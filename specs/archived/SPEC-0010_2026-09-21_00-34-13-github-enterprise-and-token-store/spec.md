# GitHub Enterprise Support and Git Token Store Seeding — Specification Document

> Generated on: 2026-09-21
> Project: mcp-fs (Rust)
> Version: 1.0
> Status: Draft
> Type: Evolution Specification
> Nature: FEAT
> Depth: L
> Depth evidence: 3 modules touched (`tools/git.rs`, `tools/git_auth.rs`, `config.rs`), public tool-contract change (3 modified schemas, 5 new tools), and a persisted schema change on `oauth_tokens`. Two independent L triggers: module count and new persisted schema. No escalation occurred during the run; scope grew from two defect fixes to full bidirectional remote sync by user decision (DEC-011), which the classification already accommodated.

---

## 1. Executive Summary

`mcp-fs` can clone a git repository from a remote, and can obtain an OAuth token through a
device flow. Both capabilities are narrower than they appear, in ways that block a consuming
project.

This specification does three things.

**It makes provider resolution correct.** Today the provider is guessed from the clone URL by
substring match (`crates/mcp-fs/src/tools/git.rs:683-691`). The guess is simultaneously too
narrow, because `github.ibm.com` does not contain the literal `github.com` and therefore never
gets a token, and too broad, because any URL whose path happens to contain `gitlab` is treated
as GitLab. Resolution moves to an explicit, boot-validated host-to-provider map with exact
hostname matching.

**It opens the token store.** `OAuthTokenStore::store_token` is public
(`crates/mcp-fs/src/git/oauth/store.rs:115`) but reachable from exactly one place: the detached
device-flow poller (`crates/mcp-fs/src/tools/git_auth.rs:199-211`). A caller already holding a
person's personal access token cannot hand it to the server, and must instead force that person
through an interactive flow for a credential they already possess. A seeding tool and a
per-person web screen are added. Token identity also becomes per-host, so one person may hold
distinct tokens for `github.com` and `github.ibm.com`.

**It completes the remote surface.** The stored token is consumed as an ordinary HTTPS password
(`crates/mcp-fs/src/tools/git.rs:1014`), so it already authorises writes. But the only
outbound remote operation that exists is clone. `git.remote_push`, `git.remote_fetch` and
`git.remote_pull` are added, sharing one credential pipeline.

The consuming project is Graph Studio, whose specification generator must read code from
enterprise-hosted repositories to cite `file:line` evidence. Without this work its code reader
is limited to public repositories on `github.com`, which caps the verdict of its own
implementability gate.

---

## 2. Current State Analysis

### 2.1 Project Overview

`mcp-fs` is a streamable-HTTP MCP server exposing a simulated multi-project filesystem, a REST
data plane at `/api/fs`, and an optional git HTTP smart server at `/git/{mount_id}/`. Server
state lives in a relational store (SQLite by default; PostgreSQL and SQL Server behind cargo
features). Blob bytes live in `infra.blob`.

Git support is gated by `git.enabled` and registers two tool families: eleven `git.*` tools and
three `git.auth*` tools (`crates/mcp-fs/src/tools/git.rs:63-282`,
`crates/mcp-fs/src/tools/git_auth.rs:60-131`). Git objects are content-addressed in the blob
store; a per-project relational index holds objects, refs and remotes.

### 2.2 Existing Specifications

| Spec | Scope |
|---|---|
| `specs/SPEC-0002_2026-09-18_17-37-46-platform-foundation/spec.md` | Server assembly, identity, safety, errors |
| `specs/SPEC-0003_2026-09-18_18-05-00-filesystem-engine/spec.md` | `core::fs_ops`, the filesystem operations |
| `specs/SPEC-0004_2026-09-18_18-30-00-rest-data-plane/spec.md` | `/api/fs` and OpenAPI |
| `specs/SPEC-0005_2026-09-18_18-55-00-multi-backend-storage/spec.md` | The relational layer and backends |
| `specs/SPEC-0006_2026-09-18_19-20-00-documents/spec.md` | Extraction, docx, symbols |
| `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md` | **Git objects, HTTP smart protocol, OAuth device flow. Owns `FR-601`–`FR-629`.** |
| `specs/SPEC-0008_2026-09-18_20-10-00-search-and-rag/spec.md` | Search tools and indexing |
| `specs/SPEC-0009_2026-09-18_20-35-00-optional-integrations/spec.md` | Optional integrations |

This specification modifies behaviour owned by the git spec. Per process rule, that document is
**not** edited; affected requirements are referenced and the deviations are recorded in
Section 13.

### 2.3 Relevant Architecture

Facts established by reading the code, each cited. Every claim below was verified; nothing here
is assumed.

**Provider detection.** `crates/mcp-fs/src/tools/git.rs:683-691` lowercases the URL and tests
`contains("github.com")` then `contains("gitlab")`, yielding `Some("github")`, `Some("gitlab")`
or `None`. `git.rs:693-706` looks up a token only when the provider is `Some`, and sets
`auth` to `"anonymous"` otherwise.

**Credential shape.** `crates/mcp-fs/src/tools/git.rs:1014` supplies
`git2::Cred::userpass_plaintext("oauth2", &t)`. This is a plain HTTPS credential with no OAuth
semantics, which is why a personal access token works identically to a device-flow token.

**Token store.** `OAuthTokenStore` keys entries `format!("{person}:{provider}")`, lowercased
(`crates/mcp-fs/src/git/oauth/store.rs:65-67`). `store_token` takes
`(person, provider, access_token, scopes, expires_at, instance_url)`
(`store.rs:115-123`) and upserts into memory then persistence (`store.rs:129-141`). `get_token`
returns the session **without checking expiry** (`store.rs:148-150`). Expiry logic exists in
`OAuthSession::is_valid_at` (`store.rs:34`) and `has_valid_token` (`store.rs:169`), neither of
which is on the clone path.

**Token persistence.** Table `oauth_tokens`, primary key `vec!["person", "provider"]`
(`crates/mcp-fs/src/git/oauth/persistence.rs:42`), columns `person` and `provider` typed
`TextKey` (`persistence.rs:33-34`). `upsert` and `delete` key on the same pair
(`persistence.rs:103-108`, `:130-135`). Only the bearer token is encrypted (`persistence.rs:3`).

**Git remotes.** Table `git_remotes` exists and is one of three git tables
(`crates/mcp-fs/src/git/db.rs:42`), declared with `volume_id`, `name`, `url`
(`db.rs:69-75`). `add_remote` (`db.rs:281`), `remove_remote` (`db.rs:287`) and `list_remotes`
(`db.rs:298`) are implemented, with a passing test proving upsert-by-name
(`db.rs:429-442`). **No production code calls `add_remote`.** `git.remote_clone`
(`git.rs:715-830`) imports objects, sets `HEAD` and `refs/heads/{branch}`, writes the working
tree and records an audit entry, and never records the remote. `git_remotes` is therefore live
schema with a tested API and zero writers.

**Remote operations.** `git2::RemoteCallbacks` is constructed exactly once in the whole tree,
at `git.rs:1009`, inside `clone_to_temp`. There is no push, fetch or pull to a remote. The
`/git/{mount_id}/` HTTP server is the inbound direction.

**Auth tools.** `git.auth` rejects any provider other than `github` or `gitlab`
(`git_auth.rs:160-162`), returns immediately with a device code, and spawns a detached poller
(`git_auth.rs:175-207`). `git.auth_status` and `git.auth_revoke` take an optional and a required
`provider` respectively (`git_auth.rs:104-131`).

**Config.** `GitConfig` (`crates/mcp-fs/src/config.rs:447-457`) carries `enabled`,
`object_format`, `anonymous_read`, `max_pack_size_mb`, `github_client_id`,
`github_client_secret_env`, `gitlab_client_id`, `gitlab_client_secret_env`,
`gitlab_instance_url`. There is **no** host-to-provider mapping of any kind.

**Boot validation.** The server holds the property that misconfiguration fails at boot rather
than on first use (`README.md:140-142`), enforced by `validate_store` calls including
`validate_store("oauth", ...)` (`config.rs:817`).

**Web session precedent.** `doc.open_editor` spawns an axum server bound to `127.0.0.1:0`
(`crates/mcp-fs/src/tools/editor.rs:208`) and returns `http://127.0.0.1:{port}`
(`editor.rs:161`, `:188`, `:293`). The `editor_id` is a UUIDv4 (`editor.rs:216`) but **the URL
carries no token**, so access control is reachability of loopback on the server host.

**Merge capability.** `Cargo.toml:67` declares `git2 = "0.20"`, a caret requirement resolved to 0.20.4 in the committed `Cargo.lock`. It provides
`MergeOptions::file_favor(FileFavor)` (`git2-0.20.4/src/merge.rs:133-136`), `FileFavor` mapping
to `GIT_MERGE_FILE_FAVOR_{NORMAL,OURS,THEIRS,UNION}` (`git2-0.20.4/src/call.rs`),
`Repository::merge_commits` (`src/repo.rs:2177`), `merge_trees` (`:2200`), `merge_base`
(`:2456`) and `graph_descendant_of` (`:2623`).

---

## 3. Scope

### 3.1 In Scope

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
- Extraction of `crates/mcp-fs/src/git/remote.rs` owning host resolution, credential supply and
  the four remote operations.
- Regeneration of `TOOL_CONTRACT.txt` and `tool-contract-golden.json`.

### 3.2 Out of Scope (Non-Goals)

- **Token refresh and rotation.** `OAuthSession` carries `expires_at`; nothing renews it.
- **Providers beyond `github`, `gitlab`, `generic`, `anonymous`.** Azure DevOps explicitly
  excluded.
- **Changing the credential shape.** `oauth2:<token>` already authorises read and write.
- **SSH transport.** Everything here is HTTPS.
- **Runtime editing of the host map.** It is file-based and boot-validated; a screen may read
  it, never write it.
- **Force push.** Backlogged with a risk analysis.
- **Pull request creation, squash merge, interactive merge conflict resolution.** Backlogged.
- **Remote management tools** (`git.remote_add`/`remove`/`list`). Backlogged.
- **A remote branch name differing from the local one.** Backlogged.
- **`git.discard_changes`.** Backlogged.
- **A pluggable `CredentialProvider` trait.** Backlogged.
- **Migrating pre-existing `oauth_tokens` rows.** They are dropped (DEC-005).

---

## 4. User Personas & Actors

| Actor | Description | Identity source |
|---|---|---|
| **Person** | A human whose identity the server verifies. Owns tokens. | RS256 JWT, `crates/mcp-fs/src/identity.rs` |
| **Calling agent** | An MCP client acting on behalf of a Person. Holds no identity of its own; every call carries the Person's identity. Graph Studio is the motivating instance. | Forwarded header or `Authorization` |
| **Operator** | Deploys and configures the server. Edits YAML, runs `migrate`. Has no implicit file access. | Filesystem and process control |
| **Platform admin** | Manages projects and membership. **Explicitly does not gain file access or access to another Person's tokens.** | RS256 JWT with admin claim |

---

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| **Host Resolution** | Mapping a remote URL to a provider and a credential policy. Boot-validated, immutable at runtime. | HostMap, HostEntry, Provider |
| **Token Custody** | Storing, retrieving, expiring and revoking a Person's credentials. Encryption at rest. | OAuthSession, OAuthTokenStore, RelationalOAuthPersistence |
| **Remote Operations** | Executing clone, push, fetch and pull against a remote, using a resolved credential. | RemotePipeline, Origin, MergeOutcome |
| **Token Self-Service** | A Person viewing and managing their own credentials through a browser. | TokenView, TokenScreenSession |

Terminology hazard this table exists to prevent: **"provider"** in Host Resolution is a
credential policy (`github`, `gitlab`, `generic`, `anonymous`), whereas **"provider"** in Token
Custody is an attribute recorded on a stored session. They carry the same values but are not the
same concept: Host Resolution decides it, Token Custody records it. **"remote"** in Remote
Operations means the stored `origin` row, never a libgit2 `Remote` handle.

---

## 5. Usage Scenarios

### SC-001: Operator configures the host-to-provider map

**Actor:** Operator
**Preconditions:** Server not running. A YAML config with a `git` section exists
(`config.rs:447-457`).
**Flow:**
1. Operator adds host-to-provider entries under `git.hosts`.
2. Operator starts the server.
3. Server validates every entry: hostname syntactically valid, provider one of the four
   accepted values, no duplicate host.
4. Server boots; the map is immutable for the process lifetime.

**Postconditions:** The map is authoritative for every remote operation. No code path mutates it.
**Exceptions:**
- **EXC-001a**: Unknown provider value → boot fails, naming the offending host and the accepted values.
- **EXC-001b**: Duplicate host entry → boot fails, naming the host.
- **EXC-001c**: Malformed host (scheme, path, port, or wildcard) → boot fails, naming the host.
- **EXC-001d**: Map absent or empty → boot succeeds; every remote operation then fails per SC-005.

**Cross-scenario notes:** Every other scenario depends on this map. An empty map makes SC-004,
SC-009, SC-011 and SC-012 all fail with the SC-005 error.

### SC-002: Calling agent seeds a token it already holds

**Actor:** Calling agent, on behalf of a Person
**Preconditions:** Server running, git enabled, caller authenticated (`ctx.person` non-empty).
Target host declared with provider `github`, `gitlab` or `generic`.
**Flow:**
1. Agent calls `git.token_set` with `host`, `token`, and optionally `expires_at`.
2. Server resolves `host` to a provider from the map.
3. Server calls `store_token` keyed `(person, host)`, encrypting at rest under `MCPFS_TOKEN_KEY`.
4. Server returns success without echoing the token.

**Postconditions:** A token exists for `(person, host)`. Remote operations against that host use
it. The value appears in no log, trace, audit entry or response.
**Exceptions:**
- **EXC-002a**: Host not declared → rejected, naming the host.
- **EXC-002b**: Empty or whitespace-only token → rejected.
- **EXC-002c**: No authenticated identity → `ERR_UNAUTHENTICATED`.
- **EXC-002d**: Token already exists for `(person, host)` → overwritten, since `store_token` upserts (`store.rs:129-141`).
- **EXC-002e**: Host declared `anonymous` → rejected; an anonymous host holds no credential.
- **EXC-002f**: Token longer than 8192 characters → rejected.
- **EXC-002g**: Configured persistence write fails → the call fails; success is not reported for a token that will not survive restart.

### SC-003: Person completes the device flow for a specific host

**Actor:** Person
**Preconditions:** Git enabled; OAuth client id configured for the provider
(`config.rs:452-456`); host declared as `github` or `gitlab`.
**Flow:**
1. Person calls `git.auth` with `provider` and optionally `host`.
2. Server validates the provider (`git_auth.rs:160-162`) and that `host` maps to it.
3. Server returns `user_code` and `verification_uri` immediately (`git_auth.rs:163-178`).
4. A detached poller runs until authorisation, refusal or code expiry (`git_auth.rs:179-222`).
5. On success the token is stored keyed `(person, host)`.

**Postconditions:** Token stored for `(person, host)`, encrypted at rest.
**Exceptions:**
- **EXC-003a**: `host` omitted → defaults to the provider's canonical public host (`github`→`github.com`, `gitlab`→`gitlab.com`).
- **EXC-003b**: `host` maps to a different provider than the one given → rejected, naming both.
- **EXC-003c**: `host` maps to `generic` or `anonymous` → rejected; the device flow exists only for `github` and `gitlab`.
- **EXC-003d**: Person never authorises → code expires, nothing stored, poller exits (`git_auth.rs:193-196`).
- **EXC-003e**: Provider is not `github`/`gitlab` → `ERR_INVALID_ARGUMENT` (existing behaviour).

### SC-004: Clone a private repository on a mapped enterprise host

**Actor:** Calling agent, on behalf of a Person
**Preconditions:** `github.ibm.com → github` declared. A valid, unexpired token stored for
`(person, github.ibm.com)`. `mount_id` names a volume the Person may write.
**Flow:**
1. Agent calls `git.remote_clone` with `mount_id` and `url = https://github.ibm.com/org/repo.git`.
2. Server authorises the Person on the mount.
3. Server extracts the hostname by **parsing** the URL, yielding `github.ibm.com`.
4. Map lookup returns provider `github`.
5. Token lookup keyed `(person, github.ibm.com)`; expiry checked.
6. Clone runs with `Cred::userpass_plaintext("oauth2", token)`.
7. Objects and refs are imported; the clone URL is recorded as remote `origin`.
8. Response reports `auth: "github"`.

**Postconditions:** Volume holds the repository. Remote `origin` is recorded. Token unchanged and
absent from every observable surface.
**Exceptions:**
- **EXC-004a**: No token stored for `(person, host)` → fails naming the host, instructing to authenticate. Not anonymous.
- **EXC-004b**: Token expired → SC-014.
- **EXC-004c**: Remote rejects the credential → transport error surfaced with the host named; the token is **not** deleted.
- **EXC-004d**: Host unreachable or TLS failure → error naming the host.
- **EXC-004e**: Person not a member of the mount → `ERR_FORBIDDEN` before any network call.
- **EXC-004f**: URL is not HTTPS → rejected.
- **EXC-004g**: URL carries userinfo → rejected.

### SC-005: Clone from an undeclared host

**Actor:** Calling agent
**Preconditions:** The URL's host appears nowhere in the map.
**Flow:**
1. Agent calls `git.remote_clone` with a URL on an undeclared host.
2. Server authorises the Person on the mount.
3. Server parses the hostname and looks it up.
4. Lookup misses.
5. Server fails, naming the host and stating it must be declared.

**Postconditions:** No network call. No clone. Volume unchanged.
**Exceptions:**
- **EXC-005a**: Host declared `anonymous` → not this scenario; the clone proceeds with no credential and `auth: "anonymous"`.
- **EXC-005b**: URL has no parseable host → rejected as malformed, a distinct error from undeclared-host.

**Cross-scenario notes:** This is a deliberate behaviour break. `git.rs:683-706` currently
returns `auth: "anonymous"` and attempts the clone. Accepted under DEC-010.

### SC-006: Person manages their tokens in the browser

**Actor:** Person
**Preconditions:** Git enabled; caller authenticated via the RS256 identity layer.
**Flow:**
1. Person navigates to the token screen route on the main server.
2. The identity layer authenticates them.
3. The page lists the hosts they hold tokens for: host, provider, and validity (`valid` or `expired`). The token value is never rendered and never sent to the browser.
4. Person seeds a token for a declared host, or revokes one.

**Postconditions:** The store reflects the change. No token value ever leaves the server.
**Exceptions:**
- **EXC-006a**: Host not declared → the page offers only declared hosts; a direct submission for an undeclared host is rejected.
- **EXC-006b**: Empty token submitted → rejected.
- **EXC-006c**: Unauthenticated request → rejected by the identity layer.
- **EXC-006d**: A Person attempts to view or modify another Person's tokens → rejected, including when the caller is a platform admin.

### SC-007: Status and revocation scoped to a host

**Actor:** Person or calling agent
**Preconditions:** Authenticated. Zero or more tokens stored.
**Flow:**
1. `git.auth_status` with optional `provider` and optional `host` reports one entry per stored `(person, host)`: host, provider, validity, expiry. Never the token.
2. `git.auth_revoke` with `provider` and optional `host` deletes exactly one stored token.

**Postconditions:** Revoked entries are gone from memory and persistence.
**Exceptions:**
- **EXC-007a**: `git.auth_revoke` with `host` omitted → defaults to the canonical public host. Never revokes across hosts.
- **EXC-007b**: Revoking a host with no stored token → success, reported as nothing revoked. Idempotent.
- **EXC-007c**: `git.auth_status` with no tokens → success, empty list.

### SC-008: Upgrade drops existing tokens

**Actor:** Operator
**Preconditions:** A prior version with rows in `oauth_tokens` keyed `(person, provider)`
(`persistence.rs:42`).
**Flow:**
1. Operator deploys the new version.
2. The schema migration changes the primary key to `(person, host)`.
3. All pre-existing rows are dropped.
4. Server boots; `git.auth_status` reports no tokens for anyone.
5. Each Person re-authenticates or reseeds once.

**Postconditions:** Table on the new key, empty of legacy rows, identical on SQLite, PostgreSQL
and SQL Server.
**Exceptions:**
- **EXC-008a**: Migration fails on one backend → boot fails naming the backend.
- **EXC-008b**: Fresh install, no existing table → migration is a no-op.
- **EXC-008c**: The `migrate` verb carries the new key; old-format source data is dropped under the same rule.

### SC-009: Push a branch to the remote

**Actor:** Calling agent, on behalf of a Person
**Preconditions:** Volume is a git repository with at least one commit and a recorded `origin`.
Target host declared. Valid unexpired token unless the host is `anonymous`. Person authorised.
**Flow:**
1. Agent calls `git.remote_push` with `mount_id` and `branch`.
2. Server authorises the Person on the mount.
3. Server reads the `origin` URL via `list_remotes` (`db.rs:298`) and parses its hostname.
4. Map lookup, token lookup, expiry check.
5. Volume is exported to a temporary working directory.
6. The named branch is pushed with `Cred::userpass_plaintext("oauth2", token)`.
7. If the branch does not exist on the remote, it is created.
8. Response reports the branch, whether it was created or updated, the resulting remote sha, and `auth`.

**Postconditions:** Remote branch points at the volume's branch tip. Volume unchanged.
**Exceptions:**
- **EXC-009a**: Named branch does not exist locally → rejected, naming the branch.
- **EXC-009b**: Remote already at that sha → success, reported as up to date. Idempotent.
- **EXC-009c**: Non-fast-forward → SC-010.
- **EXC-009d**: Remote rejects the ref → the remote's reason surfaced, with the branch named.
- **EXC-009e**: No token, expired token, or undeclared host → the SC-004, SC-005 and SC-014 failures, before any network call.
- **EXC-009f**: Volume is not a git repository → rejected.
- **EXC-009g**: No `origin` recorded → rejected, stating the volume has no origin.
- **EXC-009h**: Host declared `anonymous` → attempted with no credential; the remote's rejection is surfaced as-is.

### SC-010: Push refused as non-fast-forward

**Actor:** Calling agent
**Preconditions:** As SC-009, but the remote branch holds commits the volume does not have.
**Flow:**
1. Agent calls `git.remote_push`.
2. Auth resolves; the push is attempted.
3. The remote rejects the ref update as non-fast-forward.
4. Server fails with a distinct error stating the push was refused as non-fast-forward and that force is not supported.

**Postconditions:** Remote branch unchanged. Volume unchanged. Nothing partially applied.
**Exceptions:**
- **EXC-010a**: The remote moved between any pre-flight read and the push → the remote's rejection is authoritative and produces the same error.

### SC-011: Fetch updates refs and objects only

**Actor:** Calling agent
**Preconditions:** Volume is a cloned repository with a recorded `origin`. Host declared. Valid
unexpired token unless `anonymous`. Person authorised.
**Flow:**
1. Agent calls `git.remote_fetch` with `mount_id`.
2. Authorise; read `origin`; parse host; resolve provider, token, expiry.
3. Fetch objects into the volume's object store.
4. Update `refs/remotes/origin/*`.
5. Response reports refs updated, objects fetched, and `auth`.

**Postconditions:** New objects present. Remote-tracking refs updated. **`refs/heads/*` untouched
and the volume's files byte-for-byte unchanged.**
**Exceptions:**
- **EXC-011a**: No `origin` → rejected, stating the volume has no origin.
- **EXC-011b**: Already up to date → success, zero refs updated. Idempotent.
- **EXC-011c**: No token, expired, or undeclared host → as SC-004, SC-005, SC-014, before any network call.
- **EXC-011d**: Remote unreachable → error naming the host.

### SC-012: Pull applies a fast-forward

**Actor:** Calling agent
**Preconditions:** As SC-011, and the volume's branch is strictly behind its remote-tracking
counterpart. The volume is clean relative to the branch tip.
**Flow:**
1. Agent calls `git.remote_pull` with `mount_id` and `branch`.
2. Fetch runs exactly as SC-011.
3. Server tests whether the local tip is an ancestor of the fetched remote tip, via `graph_descendant_of`.
4. It is → the local branch ref advances to the remote tip.
5. The volume's files are updated to the new tree.
6. Response reports old sha, new sha, and files changed.

**Postconditions:** Local branch at the remote tip; volume files match that tree. No merge commit.
**Exceptions:**
- **EXC-012a**: Not a fast-forward and `on_conflict` absent → SC-013; nothing applied.
- **EXC-012b**: Already up to date → success, no change. Idempotent.
- **EXC-012c**: Volume dirty relative to the branch tip → refused, instructing the caller to commit or discard first.
- **EXC-012d**: A file write fails mid-apply → nothing applied; the ref is not advanced (atomicity, DEC-023).
- **EXC-012e**: Write quota insufficient → refused before any write.

### SC-013: Pull refused, not a fast-forward, no strategy given

**Actor:** Calling agent
**Preconditions:** Local branch holds commits the remote does not have. `on_conflict` absent.
**Flow:**
1. Fetch succeeds.
2. The ancestry test fails.
3. Server fails with a distinct error stating the pull was refused because it is not a fast-forward, and naming `on_conflict` as the way to merge.

**Postconditions:** **Fetched objects and remote-tracking refs are kept** — the fetch genuinely
happened. Local branch unchanged. Volume files unchanged.

### SC-014: Remote operation with an expired stored token

**Actor:** Calling agent or Person
**Preconditions:** A token exists for `(person, host)` whose `expires_at` is in the past.
**Flow:**
1. Clone, push, fetch or pull is called against that host.
2. Server resolves host to provider and finds the stored token.
3. The expiry test fails.
4. Server fails before any network call, naming the host and instructing re-authentication.

**Postconditions:** No network call. Volume unchanged. **The expired token remains in the store**
(DEC-020), so `git.auth_status` can report `expired` rather than absent.
**Exceptions:**
- **EXC-014a**: Token stored with no expiry → never expires; the operation proceeds.
- **EXC-014b**: Token expires during a long operation → not detected; the remote rejects it and EXC-004c applies.

### SC-015: Pull merges a diverged branch with a global strategy

**Actor:** Calling agent
**Preconditions:** Local branch and remote branch have diverged. `on_conflict` is `ours` or
`theirs`. Volume clean relative to the branch tip.
**Flow:**
1. Agent calls `git.remote_pull` with `mount_id`, `branch` and `on_conflict`.
2. Fetch runs as SC-011.
3. Ancestry test fails, so a three-way merge runs using `merge_commits` with
   `MergeOptions::file_favor` set from `on_conflict`.
4. Every conflicting file is resolved by the single global strategy. **No conflict markers are
   written into the volume, ever.**
5. A merge commit with two parents is created, authored by the authenticated Person, with an
   auto-generated message recording the strategy.
6. The volume's files are updated to the merged tree, atomically.

**Postconditions:** Branch has a merge commit whose parents are the previous local tip and the
fetched remote tip. The branch is now strictly ahead of the remote, so `git.remote_push`
fast-forwards cleanly.
**Exceptions:**
- **EXC-015a**: The merge produces no conflicts at all → a merge commit is still created.
- **EXC-015b**: A file write fails → nothing applied; the ref is not advanced.
- **EXC-015c**: `on_conflict` is any value other than `ours` or `theirs` → rejected, naming the accepted values.
- **EXC-015d**: Volume dirty → refused, as EXC-012c, before any merge.
- **EXC-015e**: Write quota insufficient for the merged tree → refused before any write.
- **EXC-015f**: The merge is degenerate because the local branch is actually an ancestor → handled as SC-012, a fast-forward, with no merge commit.

---

## 6. Functional Requirements

EARS notation is mandatory. Forbidden modals: `should`, `may`, `could`, `might`, `would`.

### New Requirements

#### FR-NEW-001 [EARS-U]: The host-to-provider map exists in configuration
> The mcp-fs server SHALL expose a configuration key `git.hosts` mapping a hostname to one of the providers `github`, `gitlab`, `generic` or `anonymous`.

- **Inputs:** YAML under `git.hosts`, e.g. `github.ibm.com: github`.
- **Outputs:** An in-memory map available to every remote operation.
- **Business Rules:** The key is the bare hostname. The value is exactly one of the four accepted strings, lowercase.
- **Priority:** Must-have
- **Rationale:** Replaces substring detection at `crates/mcp-fs/src/tools/git.rs:683-691`.

#### FR-NEW-002 [EARS-E]: Boot rejects an unknown provider value
> WHEN the server starts and any `git.hosts` entry names a provider outside `github`, `gitlab`, `generic`, `anonymous`, THE mcp-fs server SHALL refuse to start, naming the offending host and listing the accepted values.

- **Inputs:** `git.hosts` with e.g. `github.ibm.com: githib`.
- **Outputs:** Boot failure, non-zero exit, message naming `github.ibm.com`.
- **Business Rules:** Consistent with the existing property that misconfiguration fails at boot (`README.md:140-142`).
- **Priority:** Must-have

#### FR-NEW-003 [EARS-E]: Boot rejects a duplicate host
> WHEN the server starts and the same hostname appears more than once in `git.hosts`, THE mcp-fs server SHALL refuse to start, naming the duplicated host.

- **Inputs:** Two entries for `github.com`.
- **Outputs:** Boot failure naming `github.com`.
- **Business Rules:** Precedence between duplicates is never inferred.
- **Priority:** Must-have

#### FR-NEW-004 [EARS-E]: Boot rejects a malformed host
> WHEN the server starts and any `git.hosts` key contains a scheme, a path, a port, or a wildcard character, THE mcp-fs server SHALL refuse to start, naming the offending key.

- **Inputs:** `https://github.com`, `github.com/org`, `github.com:443`, `*.acme.corp`.
- **Outputs:** Boot failure naming the key.
- **Business Rules:** Wildcards are rejected rather than supported, because precedence between overlapping patterns is exactly the ambiguity this requirement removes.
- **Priority:** Must-have

#### FR-NEW-005 [EARS-UB]: The map is immutable at runtime
> The mcp-fs server SHALL NOT expose any tool, route or API that modifies `git.hosts` after boot.

- **Inputs:** None.
- **Outputs:** None.
- **Business Rules:** A screen reads the map; nothing writes it.
- **Priority:** Must-have
- **Rationale:** A hot-editable trust routing table breaks the boot-validation property.

#### FR-NEW-006 [EARS-E]: Hostname resolution is by parsing and exact match
> WHEN resolving a remote URL to a provider, THE mcp-fs server SHALL extract the hostname by parsing the URL and SHALL match it against `git.hosts` by exact, case-insensitive equality.

- **Inputs:** `https://github.ibm.com/org/repo.git`; `https://exemple.test/mirrors/mygitlab-mirror.git`.
- **Outputs:** `github` for the first. No match for the second.
- **Business Rules:** Substring matching is prohibited. `github.ibm.com` matches only an entry spelled `github.ibm.com`. A URL whose path contains `gitlab` does not resolve to `gitlab`.
- **Priority:** Must-have
- **Rationale:** Directly fixes both halves of the defect at `git.rs:683-691`.

#### FR-NEW-007 [EARS-O]: An undeclared host is an error
> IF a remote URL's hostname is absent from `git.hosts`, THEN THE mcp-fs server SHALL fail the operation before any network call, naming the host and stating that it must be declared in `git.hosts`.

- **Inputs:** A clone of `https://git.unknown.test/a.git` with no matching entry.
- **Outputs:** `ERR_INVALID_ARGUMENT` naming `git.unknown.test`. No socket opened.
- **Business Rules:** Anonymous access is never inferred. It is declared.
- **Priority:** Must-have
- **Rationale:** DEC-010. This is a deliberate behaviour break from `git.rs:693-706`.

#### FR-NEW-008 [EARS-O]: A host declared anonymous carries no credential
> IF a remote URL's hostname resolves to provider `anonymous`, THEN THE mcp-fs server SHALL perform the operation with no credential and SHALL report `auth` as `"anonymous"`.

- **Inputs:** `public.example.org: anonymous` and a clone of `https://public.example.org/a.git`.
- **Outputs:** Clone succeeds; response contains `"auth": "anonymous"`.
- **Business Rules:** No token lookup occurs for an anonymous host.
- **Priority:** Must-have

#### FR-NEW-009 [EARS-U]: Token identity is per host
> The mcp-fs server SHALL key every stored credential on the pair `(person, host)`, case-insensitively on both parts, holding at most one token per pair.

- **Inputs:** `store_token("alice@test.com", "github.ibm.com", …)`.
- **Outputs:** An entry retrievable by `("ALICE@TEST.COM", "GitHub.IBM.com")`.
- **Business Rules:** Replaces `format!("{person}:{provider}")` at `crates/mcp-fs/src/git/oauth/store.rs:65-67`. The provider becomes an attribute of the stored session, not part of its identity.
- **Priority:** Must-have
- **Rationale:** One Person holds distinct tokens for `github.com` and `github.ibm.com`, both provider `github`.

#### FR-NEW-010 [EARS-U]: The persisted primary key is per host
> The mcp-fs server SHALL persist credentials in `oauth_tokens` with primary key `(person, host)` on SQLite, PostgreSQL and SQL Server alike.

- **Inputs:** Schema definition.
- **Outputs:** A table whose key columns are `person` and `host`, both `TextKey`.
- **Business Rules:** Replaces `vec!["person", "provider"]` at `crates/mcp-fs/src/git/oauth/persistence.rs:42`. `host` is `TextKey` bounded, because SQL Server cannot index `NVARCHAR(MAX)`.
- **Priority:** Must-have

#### FR-NEW-011 [EARS-E]: Migration drops legacy rows
> WHEN the server starts against an `oauth_tokens` table keyed `(person, provider)`, THE mcp-fs server SHALL drop every pre-existing row and rebuild the table keyed `(person, host)`.

- **Inputs:** An existing table with rows for `(alice@test.com, github)`.
- **Outputs:** A table on the new key containing zero rows.
- **Business Rules:** No host is inferred from a legacy provider value. Every Person re-authenticates once.
- **Priority:** Must-have
- **Rationale:** DEC-005. Inferring `github.com` from `provider=github` would write a guess into a primary key.

#### FR-NEW-012 [EARS-E]: The migrate verb carries the new key
> WHEN `mcp-fs migrate` copies relational state between backends, THE mcp-fs server SHALL copy `oauth_tokens` under the `(person, host)` key and SHALL drop rows found in the legacy format.

- **Inputs:** `mcp-fs migrate --from sqlite.yaml --to postgres.yaml`.
- **Outputs:** A destination table on the new key.
- **Priority:** Must-have

#### FR-NEW-013 [EARS-E]: The seeding tool stores a supplied token
> WHEN `git.token_set` is called with a declared `host` and a non-empty `token`, THE mcp-fs server SHALL store that token for `(person, host)` and SHALL return success without echoing the token.

- **Inputs:** `{"host": "github.ibm.com", "token": "ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH", "expires_at": null}`.
- **Outputs:** `{"host": "github.ibm.com", "provider": "github", "stored": true, "persistent": true}`. The response contains no substring of the token.
- **Business Rules:** The provider is resolved from the map, never supplied by the caller. Calls `OAuthTokenStore::store_token` (`crates/mcp-fs/src/git/oauth/store.rs:115`).
- **Priority:** Must-have
- **Rationale:** `store_token` is public but reachable only from the device-flow poller (`crates/mcp-fs/src/tools/git_auth.rs:199-211`).

#### FR-NEW-014 [EARS-O]: An absent expiry means non-expiring
> IF `git.token_set` is called without `expires_at`, THEN THE mcp-fs server SHALL store the token as non-expiring.

- **Inputs:** `expires_at` omitted or null.
- **Outputs:** A stored session that never fails the expiry test of FR-NEW-018.
- **Business Rules:** A personal access token can genuinely have no expiry; fabricating one would wrongly trigger FR-NEW-018.
- **Priority:** Must-have

#### FR-NEW-015 [EARS-O]: The seeding tool rejects an unusable token
> IF `git.token_set` receives a token that is empty, whitespace-only, or longer than 8192 characters, THEN THE mcp-fs server SHALL reject the call with `ERR_INVALID_ARGUMENT` and SHALL store nothing.

- **Inputs:** `""`, `"   "`, or a 8193-character string.
- **Outputs:** `ERR_INVALID_ARGUMENT`. `git.auth_status` reports no token for that host.
- **Business Rules:** 8192 is the bound; 8192 exactly is accepted, 8193 is rejected.
- **Priority:** Must-have

#### FR-NEW-016 [EARS-O]: The seeding tool rejects an undeclared or anonymous host
> IF `git.token_set` names a host absent from `git.hosts`, or a host declared `anonymous`, THEN THE mcp-fs server SHALL reject the call with `ERR_INVALID_ARGUMENT`, naming the host.

- **Inputs:** `{"host": "git.unknown.test", "token": "ghp_x"}`; `{"host": "public.example.org", "token": "ghp_x"}` where that host is `anonymous`.
- **Outputs:** `ERR_INVALID_ARGUMENT` naming the host. Nothing stored.
- **Business Rules:** An anonymous host holds no credential by definition.
- **Priority:** Must-have

#### FR-NEW-017 [EARS-O]: Seeding fails loudly when configured persistence fails
> IF `git.token_set` is called while relational persistence is configured and the persistence write fails, THEN THE mcp-fs server SHALL fail the call and SHALL NOT report success.

- **Inputs:** A seeding call with the backing store unreachable.
- **Outputs:** `ERR_INTERNAL_ERROR` or the retryable database error; the response never claims the token was stored.
- **Business Rules:** `store_token` writes memory first, then persistence (`store.rs:129-141`). Reporting success for a token that vanishes on restart is a silent data-loss defect.
- **Priority:** Must-have

#### FR-NEW-018 [EARS-E]: Expiry is enforced before the network
> WHEN a remote operation resolves a stored token whose `expires_at` is in the past, THE mcp-fs server SHALL fail the operation before opening any network connection, naming the host and instructing the person to re-authenticate.

- **Inputs:** A stored token for `(alice@test.com, github.ibm.com)` with `expires_at` one hour in the past, then `git.remote_clone`.
- **Outputs:** An error naming `github.ibm.com` and referencing re-authentication. No socket is opened.
- **Business Rules:** The clone path currently uses `get_token` (`crates/mcp-fs/src/tools/git.rs:704`), which ignores expiry (`store.rs:148-150`), unlike `has_valid_token` (`store.rs:169`).
- **Priority:** Must-have

#### FR-NEW-019 [EARS-UB]: An expired token is not deleted
> The mcp-fs server SHALL NOT delete a stored token as a consequence of detecting that it has expired.

- **Inputs:** A remote operation that fails per FR-NEW-018.
- **Outputs:** `git.auth_status` still lists the host, reporting `expired`.
- **Business Rules:** Deletion on a read path is a surprising side effect and loses the distinction between expired and absent.
- **Priority:** Must-have

#### FR-NEW-020 [EARS-E]: Clone records the origin remote
> WHEN `git.remote_clone` completes successfully, THE mcp-fs server SHALL record the clone URL as the remote named `origin` for that volume.

- **Inputs:** A clone of `https://github.ibm.com/org/repo.git` into mount `proj-a`.
- **Outputs:** `list_remotes` for `proj-a` returns `[("origin", "https://github.ibm.com/org/repo.git")]`.
- **Business Rules:** Uses the existing `add_remote` (`crates/mcp-fs/src/git/db.rs:281`). Re-cloning the same volume from a different URL replaces the row, since `add_remote` upserts by name (`db.rs:429-442`).
- **Priority:** Must-have
- **Rationale:** `git_remotes` is live schema with a tested API and, today, no writer at all: `git.remote_clone` (`git.rs:715-830`) never calls it. Push, fetch and pull have no other source for the URL.

#### FR-NEW-021 [EARS-O]: A volume without an origin cannot reach a remote
> IF `git.remote_push`, `git.remote_fetch` or `git.remote_pull` is called on a volume with no recorded `origin`, THEN THE mcp-fs server SHALL reject the call, stating that the volume has no origin remote.

- **Inputs:** A volume created by `git.init`, then `git.remote_push`.
- **Outputs:** `ERR_INVALID_ARGUMENT` stating the volume has no `origin`. No network call.
- **Business Rules:** These tools take no `url` parameter. Remote management is out of scope.
- **Priority:** Must-have

#### FR-NEW-022 [EARS-E]: Push sends one named branch
> WHEN `git.remote_push` is called with a `branch` that exists locally, THE mcp-fs server SHALL push that branch to `origin` under the same name and SHALL report the branch, whether it was created or updated, the resulting remote sha, and the resolved `auth`.

- **Inputs:** `{"mount_id": "proj-a", "branch": "main"}`.
- **Outputs:** `{"branch": "main", "created": false, "remote_sha": "<40 hex>", "auth": "github"}`.
- **Business Rules:** The remote branch name always equals the local one.
- **Priority:** Must-have

#### FR-NEW-023 [EARS-O]: Push creates a branch absent on the remote
> IF the branch named in `git.remote_push` does not exist on the remote, THEN THE mcp-fs server SHALL create it and SHALL report `created` as true.

- **Inputs:** Local branch `feature/x` absent on the remote.
- **Outputs:** `{"branch": "feature/x", "created": true, …}`; the remote then holds `refs/heads/feature/x`.
- **Priority:** Must-have

#### FR-NEW-024 [EARS-E]: A non-fast-forward push is refused
> WHEN the remote rejects a push because it is not a fast-forward, THE mcp-fs server SHALL fail with a distinct error stating that the push was refused as non-fast-forward and that force is not supported.

- **Inputs:** A remote branch holding a commit the volume lacks.
- **Outputs:** A dedicated error distinguishable from a credential failure and from a protected-branch rejection. The remote branch is unchanged.
- **Business Rules:** The remote is authoritative on fast-forwardness. No local pre-flight check overrides it.
- **Priority:** Must-have

#### FR-NEW-025 [EARS-O]: An up-to-date push succeeds
> IF the remote branch already points at the sha being pushed, THEN THE mcp-fs server SHALL report success with an up-to-date indication rather than an error.

- **Inputs:** `git.remote_push` called twice in succession with no intervening commit.
- **Outputs:** Both calls succeed; the second reports `up_to_date` true.
- **Business Rules:** Push is idempotent.
- **Priority:** Must-have

#### FR-NEW-026 [EARS-E]: Fetch updates objects and remote-tracking refs
> WHEN `git.remote_fetch` is called on a volume with an `origin`, THE mcp-fs server SHALL download new objects and update `refs/remotes/origin/*`, and SHALL report the refs updated and the object count.

- **Inputs:** `{"mount_id": "proj-a"}` with the remote two commits ahead.
- **Outputs:** `refs/remotes/origin/main` at the remote tip; response lists the updated ref.
- **Priority:** Must-have

#### FR-NEW-027 [EARS-UB]: Fetch never changes local refs or files
> The mcp-fs server SHALL NOT modify `refs/heads/*` or any file in the volume as a consequence of `git.remote_fetch`.

- **Inputs:** A fetch against a remote that is ahead.
- **Outputs:** Every path in the volume is byte-for-byte identical before and after. `refs/heads/main` unchanged.
- **Business Rules:** Fetch is the safe primitive. Nothing a person sees in the filesystem moves.
- **Priority:** Must-have

#### FR-NEW-028 [EARS-E]: Pull applies a fast-forward
> WHEN `git.remote_pull` is called and the local branch tip is an ancestor of the fetched remote tip, THE mcp-fs server SHALL advance the local branch to the remote tip, update the volume's files to that tree, and report the old sha, the new sha and the count of files changed.

- **Inputs:** `{"mount_id": "proj-a", "branch": "main"}` with the local branch two commits behind.
- **Outputs:** `{"old_sha": …, "new_sha": …, "files_changed": 3, "merged": false}`.
- **Business Rules:** Ancestry is tested with `Repository::graph_descendant_of` (`git2-0.20.4/src/repo.rs:2623`). No merge commit is created.
- **Priority:** Must-have

#### FR-NEW-029 [EARS-O]: Pull refuses a dirty volume
> IF the volume's files differ from the current branch tip when `git.remote_pull` is called, THEN THE mcp-fs server SHALL refuse the pull before fetching, instructing the caller to commit or discard the changes first.

- **Inputs:** `fs.write` to `/README.md`, then `git.remote_pull`.
- **Outputs:** A dedicated error. The volume is untouched and no fetch occurs.
- **Business Rules:** The volume is the working tree; there is no staging area. Overwriting uncommitted edits silently is prohibited.
- **Priority:** Must-have

#### FR-NEW-030 [EARS-O]: A diverged pull without a strategy is refused
> IF the local branch is not an ancestor of the fetched remote tip and `on_conflict` is absent, THEN THE mcp-fs server SHALL refuse the pull with a distinct error naming `on_conflict` as the way to merge, and SHALL retain the fetched objects and updated remote-tracking refs.

- **Inputs:** Diverged branches, `on_conflict` omitted.
- **Outputs:** A dedicated error. `refs/remotes/origin/main` is updated; `refs/heads/main` and the volume's files are unchanged.
- **Business Rules:** The fetch genuinely happened and its results are kept. Refusal applies only to the apply step. Divergence is never resolved implicitly: the caller states a strategy or the operation stops.
- **Priority:** Must-have

#### FR-NEW-031 [EARS-O]: A diverged pull with a strategy merges
> IF the local branch is not an ancestor of the fetched remote tip and `on_conflict` is `ours` or `theirs`, THEN THE mcp-fs server SHALL perform a three-way merge resolving every conflicting file by that single strategy, and SHALL create a merge commit with two parents.

- **Inputs:** `{"mount_id": "proj-a", "branch": "main", "on_conflict": "theirs"}` with both sides having edited `/src/lib.rs`.
- **Outputs:** `{"merged": true, "strategy": "theirs", "merge_commit": "<40 hex>", "conflicts_resolved": 1}`. `/src/lib.rs` holds the remote's content.
- **Business Rules:** Implemented with `MergeOptions::file_favor` (`git2-0.20.4/src/merge.rs:133-136`) set to `FileFavor::Ours` or `FileFavor::Theirs`, and `Repository::merge_commits` (`src/repo.rs:2177`). The merge commit's first parent is the previous local tip, its second the fetched remote tip.
- **Priority:** Must-have

#### FR-NEW-032 [EARS-UB]: No conflict markers ever enter the volume
> The mcp-fs server SHALL NOT write conflict markers, index conflict entries, or any partially merged representation into the volume.

- **Inputs:** Any merge under FR-NEW-031.
- **Outputs:** No file in the volume contains `<<<<<<<`, `=======` or `>>>>>>>` introduced by the merge.
- **Business Rules:** The global strategy resolves every conflict; there is nothing left to represent.
- **Priority:** Must-have

#### FR-NEW-033 [EARS-O]: An invalid conflict strategy is rejected
> IF `on_conflict` holds any value other than `ours` or `theirs`, THEN THE mcp-fs server SHALL reject the call with `ERR_INVALID_ARGUMENT`, naming the accepted values.

- **Inputs:** `{"on_conflict": "union"}`, `{"on_conflict": "OURS"}`, `{"on_conflict": ""}`.
- **Outputs:** `ERR_INVALID_ARGUMENT` listing `ours` and `theirs`. Nothing fetched is applied.
- **Business Rules:** Matching is exact and lowercase, mirroring `git.auth`'s exact provider check (`git_auth.rs:160-162`).
- **Priority:** Must-have

#### FR-NEW-034 [EARS-U]: The merge commit records author and strategy
> The mcp-fs server SHALL author every merge commit with the authenticated person's identity and SHALL set its message to an auto-generated string naming the source ref, the target branch and the conflict strategy applied.

- **Inputs:** A merge by `alice@test.com` of `origin/main` into `main` with `theirs`.
- **Outputs:** Commit author and committer are `alice@test.com`; message is `Merge origin/main into main (conflicts resolved: theirs)`.
- **Business Rules:** No caller-supplied message parameter exists. The strategy stays visible in `git.log` permanently.
- **Priority:** Must-have

#### FR-NEW-035 [EARS-UB]: Pull is atomic
> The mcp-fs server SHALL NOT advance a branch ref when any part of applying the resulting tree to the volume fails.

- **Inputs:** A pull in which one file write fails.
- **Outputs:** `refs/heads/main` unchanged; every volume file unchanged; an error naming the failing path.
- **Business Rules:** Deliberately unlike `git.remote_clone`, which tolerates per-file failures and reports them in `skipped` (`crates/mcp-fs/src/tools/git.rs:785-791`). A half-applied clone is recoverable by re-cloning into a fresh volume; a branch advanced over stale files leaves the volume silently inconsistent with its own HEAD.
- **Priority:** Must-have

#### FR-NEW-036 [EARS-E]: Pull and merge charge the write quota first
> WHEN `git.remote_pull` computes the tree it will apply, THE mcp-fs server SHALL charge the session write quota before writing any file, on the basis fixed by FR-NEW-069, and SHALL refuse the operation without writing anything when the quota is insufficient.

- **Inputs:** A pull whose tree exceeds `safety.write_quota_bytes`.
- **Outputs:** A quota error; no file written; the ref not advanced.
- **Business Rules:** The basis of the charge is the changed blobs only, fixed by FR-NEW-069. This differs deliberately from clone, which charges its whole tree (`git.rs:778-782`) because on a clone every file genuinely is new.
- **Priority:** Must-have

#### FR-NEW-037 [EARS-U]: The token screen is served by the main server behind identity
> The mcp-fs server SHALL serve the per-person token screen as a route on the main HTTP server, authenticated by the existing RS256 identity layer.

- **Inputs:** A browser request carrying a valid RS256 JWT.
- **Outputs:** The screen, scoped to the authenticated person.
- **Business Rules:** Not a tool-spawned loopback session. The `doc.open_editor` precedent binds `127.0.0.1` with no token in the URL (`crates/mcp-fs/src/tools/editor.rs:208`, `:293`), which is unauthenticated against other local processes and unreachable when the server is remote.
- **Priority:** Must-have
- **Rationale:** DEC-028, superseding DEC-007.

#### FR-NEW-038 [EARS-E]: The screen lists held hosts without values
> WHEN a person opens the token screen, THE mcp-fs server SHALL list every host for which that person holds a token, with its provider and a validity of `valid` or `expired`, and SHALL NOT transmit any token value to the browser.

- **Inputs:** A person holding tokens for `github.com` (valid) and `github.ibm.com` (expired).
- **Outputs:** Two rows. The HTTP response body contains no substring of either token.
- **Priority:** Must-have

#### FR-NEW-039 [EARS-E]: The screen seeds and revokes
> WHEN a person submits a token for a declared host, or requests revocation of a held host, THE mcp-fs server SHALL apply the change under the same rules as `git.token_set` and `git.auth_revoke`.

- **Inputs:** A seed for `github.ibm.com`; a revoke of `github.com`.
- **Outputs:** The store reflects both; `git.auth_status` agrees with the screen.
- **Business Rules:** The screen is a second adapter over the same operations, never a second implementation.
- **Priority:** Must-have

#### FR-NEW-040 [EARS-UB]: The screen never exposes another person's tokens
> The mcp-fs server SHALL NOT allow any person, including a platform admin, to view, seed or revoke a token belonging to a different person through the token screen.

- **Inputs:** `bob@test.com`, holding the platform admin claim, requesting `alice@test.com`'s tokens.
- **Outputs:** `ERR_FORBIDDEN`; no data about alice's tokens in the response.
- **Business Rules:** Platform admin manages projects and membership and gains no implicit access.
- **Priority:** Must-have

#### FR-NEW-041 [EARS-O]: Non-HTTPS remote URLs are rejected
> IF a remote URL's scheme is not `https`, THEN THE mcp-fs server SHALL reject the operation with `ERR_INVALID_ARGUMENT`, naming the scheme.

- **Inputs:** `ssh://git@github.com/o/r.git`, `git://github.com/o/r.git`, `file:///tmp/r`, `http://github.com/o/r.git`.
- **Outputs:** `ERR_INVALID_ARGUMENT` for each. No network call, no filesystem access.
- **Business Rules:** SSH is a non-goal; `file://` would let a caller read the server's filesystem.
- **Priority:** Must-have

#### FR-NEW-042 [EARS-O]: Remote URLs carrying userinfo are rejected
> IF a remote URL contains a userinfo component, THEN THE mcp-fs server SHALL reject the operation with `ERR_INVALID_ARGUMENT` and SHALL NOT include the URL in the error message.

- **Inputs:** `https://alice:ghp_secret@github.com/o/r.git`.
- **Outputs:** `ERR_INVALID_ARGUMENT` naming the host only. The credential appears in no log, trace or audit entry, and the URL is not stored as `origin`.
- **Business Rules:** Such a URL would otherwise be recorded as `origin` and later emitted in audit entries, which record the URL today (`git.rs:794-800`).
- **Priority:** Must-have

#### FR-NEW-043 [EARS-UB]: A token never appears on any observable surface
> The mcp-fs server SHALL NOT include a token value in a tool response, a tracing record, an audit entry, an error message, or an HTTP response body.

- **Inputs:** Every operation in this specification, including failures.
- **Outputs:** No emitted string contains a stored token.
- **Business Rules:** Extends the existing redaction discipline, which already covers `Debug` output (`crates/mcp-fs/src/git/oauth/store.rs:41`, test at `:285`).
- **Priority:** Must-have

#### FR-NEW-044 [EARS-E]: Remote operations are bounded by a timeout
> WHEN a remote operation exceeds `git.remote_timeout_secs`, THE mcp-fs server SHALL abort it and fail with a distinct timeout error naming the host, releasing the per-repository write lock.

- **Inputs:** `git.remote_timeout_secs: 5` and a remote that never responds.
- **Outputs:** A timeout error naming the host within approximately five seconds. A subsequent operation on the same volume proceeds.
- **Business Rules:** The key defaults to 120 and applies to clone, push, fetch and pull.
- **Priority:** Must-have

#### FR-NEW-045 [EARS-U]: The frozen tool contract is regenerated
> The mcp-fs server SHALL expose, and the frozen contract SHALL record, the four new tools `git.token_set`, `git.remote_push`, `git.remote_fetch` and `git.remote_pull`, together with the three modified schemas `git.auth`, `git.auth_status` and `git.auth_revoke`, bringing the contract to 63 tools.

- **Inputs:** `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current`.
- **Outputs:** Updated `TOOL_CONTRACT.txt` and `tool-contract-golden.json`; all three contract tests pass.
- **Business Rules:** The golden file is regenerated, never hand-edited. The token screen is an HTTP route and SHALL NOT appear in the tool contract.
- **Priority:** Must-have


#### FR-NEW-046 [EARS-U]: The token screen route names are fixed
> The mcp-fs server SHALL serve the token screen at `GET /app/tokens`, SHALL accept seeding at `POST /app/tokens` with the `application/x-www-form-urlencoded` fields `host` and `token`, and SHALL accept revocation at `POST /app/tokens/revoke` with the field `host`.

- **Inputs:** `GET /app/tokens`; `POST /app/tokens` body `host=github.ibm.com&token=ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH`; `POST /app/tokens/revoke` body `host=github.ibm.com`.
- **Outputs:** `200 text/html` for the `GET`; `303 See Other` with `Location: /app/tokens` on a successful `POST`; `400 application/json {"error":"ERR_INVALID_ARGUMENT","detail":"..."}` on a rejected host or token; `401` when unauthenticated.
- **Business Rules:** The three routes are registered only when `git.enabled` is true. Both `POST` routes delegate to the same functions as `git.token_set` and `git.auth_revoke` (FR-NEW-039), never to a second implementation.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-001 and resolves what Section 15 previously left as a TBD. A public HTTP surface an agent would otherwise have to invent.

#### FR-NEW-047 [EARS-E]: The token screen resolves identity from three sources
> WHEN a request reaches a token screen route, THE mcp-fs server SHALL resolve the person from the configured forwarded header, then the `Authorization` header, then the `mcpfs_token` cookie, and SHALL reject the request with HTTP 401 when none yields a verified identity.

- **Inputs:** A request carrying `X-Forwarded-Authorization: Bearer <jwt>`; one carrying `Authorization: Bearer <jwt>`; one carrying `Cookie: mcpfs_token=<jwt>`; one carrying none.
- **Outputs:** The screen for the first three; `401 {"error":"ERR_UNAUTHENTICATED","detail":"..."}` for the fourth.
- **Business Rules:** The cookie is read only: the server never sets, refreshes or clears it. The JWT in the cookie is verified exactly as a header bearer token is, by `IdentityResolver`. No route outside `/app/tokens*` gains cookie acceptance.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-002. A browser cannot attach an `Authorization` header to a top-level navigation, so the header-only resolution at `crates/mcp-fs/src/identity.rs:188-193` makes the screen unreachable by the only client that can use it.

#### FR-NEW-048 [EARS-O]: The token screen rejects cross-origin submissions
> IF a `POST` to a token screen route is authenticated by the `mcpfs_token` cookie and does not carry a matching anti-forgery token, THEN THE mcp-fs server SHALL reject it with HTTP 403 and SHALL NOT modify any stored credential.

- **Inputs:** A `POST /app/tokens` from another origin carrying the cookie and no anti-forgery field; a `POST` from the rendered screen carrying the field issued with that page.
- **Outputs:** `403` and no state change for the first; success for the second.
- **Business Rules:** `GET /app/tokens` issues a single-use anti-forgery token bound to the authenticated person, which both `POST` routes require. The server never emits the cookie (FR-NEW-059), so this defence rests on the anti-forgery token alone, not on cookie attributes. A request authenticated by a header rather than the cookie carries no ambient credential and is exempt.
- **Priority:** Must-have
- **Rationale:** A cookie is attached by the browser to any request to this origin, including one triggered by another site. Without this, FR-NEW-047 turns token seeding and revocation into cross-site-forgeable operations.

#### FR-NEW-049 [EARS-U]: The four remote failure modes carry fixed codes and message prefixes
> The mcp-fs server SHALL report a non-fast-forward push as `ERR_INVALID_ARGUMENT` with the message prefix `push refused: not a fast-forward`, a dirty volume on pull as `ERR_INVALID_ARGUMENT` with the prefix `pull refused: volume has uncommitted changes`, a diverged pull with no strategy as `ERR_INVALID_ARGUMENT` with the prefix `pull refused: not a fast-forward`, and a remote timeout as `ERR_INTERNAL_ERROR` with the prefix `remote timeout`.

- **Inputs:** The four failures of FR-NEW-024, FR-NEW-029, FR-NEW-030 and FR-NEW-044.
- **Outputs:** Exactly those code and prefix pairs, each followed by the host or branch the owning requirement names.
- **Business Rules:** No new `ERR_*` constant is added; the set at `crates/mcp-fs/src/errors.rs:9-22` is closed. The prefixes are stable and are what E2E-NEW-076, E2E-NEW-120, E2E-NEW-126 and E2E-NEW-155 assert.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-003. "Distinct error" is satisfiable by a new code or by a message string; a client distinguishing failures observes a different contract in each case.

#### FR-NEW-050 [EARS-U]: The fetch response shape is fixed
> The mcp-fs server SHALL return from `git.remote_fetch` an object with `refs_updated`, an array of `{"ref": String, "old_sha": String|null, "new_sha": String}`; `refs_stale`, an array of ref names; `objects_fetched`, an integer; `up_to_date`, a boolean; and `auth`, a string.

- **Inputs:** A fetch bringing `refs/remotes/origin/main` forward two commits.
- **Outputs:** `{"refs_updated":[{"ref":"refs/remotes/origin/main","old_sha":"<40 hex>","new_sha":"<40 hex>"}],"refs_stale":[],"objects_fetched":7,"up_to_date":false,"auth":"github"}`.
- **Business Rules:** `old_sha` is null for a newly created remote-tracking ref. `up_to_date` is true exactly when `refs_updated` is empty.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-004. Push and pull spell their responses; fetch described its own in prose.

#### FR-NEW-051 [EARS-U]: The modified auth tool response shapes are fixed
> The mcp-fs server SHALL return from `git.auth_status` the object `{"statuses":[{"host":String,"provider":String,"validity":"valid"|"expired","expires_at":String|null,"scopes":[String]}]}` for every call, and SHALL return from `git.auth_revoke` the object `{"host":String,"provider":String|null,"revoked":Boolean}`.

- **Inputs:** `git.auth_status {}` for a person holding a valid `github.com` token and an expired `github.ibm.com` token; `git.auth_revoke {"host":"git.unknown.test"}`.
- **Outputs:** Two entries in `statuses`, ordered by `host` ascending; `{"host":"git.unknown.test","provider":null,"revoked":false}`.
- **Business Rules:** The `authenticated` key is removed. The single-provider answer no longer carries a shape distinct from the list. `expires_at` is serialized by the existing `round_trip_iso` helper and is null for a non-expiring token. A host with no stored token is never listed.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-005. The current shape is `authenticated: bool` plus `provider`, and it differs between the single and list answers, so "reports per host" did not determine the response.

#### FR-NEW-052 [EARS-E]: `expires_at` is an RFC 3339 string on input
> WHEN `git.token_set` receives a non-null `expires_at`, THE mcp-fs server SHALL parse it as an RFC 3339 timestamp, SHALL convert it to UTC, and SHALL reject any other form with `ERR_INVALID_ARGUMENT` naming the parameter.

- **Inputs:** `"2027-01-01T00:00:00Z"`; `"2027-01-01T01:00:00+01:00"`; `1798761600`; `"tomorrow"`.
- **Outputs:** The first two store the same instant; the last two are rejected.
- **Business Rules:** An `expires_at` already in the past is accepted and stored; the token then reports `expired` per FR-MOD-003 rather than being rejected at seeding.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-006. The stored type is `DateTime<Utc>` and the codebase serializes with a deliberately non-standard seven-digit fractional form, so the accepted input form had three defensible readings.

#### FR-NEW-053 [EARS-U]: `auth` always reports the resolved provider
> The mcp-fs server SHALL set the `auth` field of every `git.remote_clone`, `git.remote_push`, `git.remote_fetch` and `git.remote_pull` response to the resolved provider string, one of `github`, `gitlab`, `generic` or `anonymous`.

- **Inputs:** A clone of `https://git.acme.internal/o/r.git`, declared `generic`, with a seeded token.
- **Outputs:** `"auth": "generic"`.
- **Business Rules:** `anonymous` is reported only for a host declared `anonymous`, never as a fallback (FR-NEW-007).
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-007. The value was specified for `anonymous` and `github` only.

#### FR-NEW-054 [EARS-U]: One module owns the remote pipeline
> The mcp-fs server SHALL implement host resolution, URL validation, credential supply, timeout and the four remote operations exactly once, in `crates/mcp-fs/src/git/remote.rs`, and `crates/mcp-fs/src/tools/git.rs` SHALL call into it without reimplementing any of those five steps.

- **Inputs:** The source tree after implementation.
- **Outputs:** `git2::RemoteCallbacks` is constructed in exactly one file, as it is today; `git.hosts` is read in exactly one file.
- **Business Rules:** Mirrors the standing rule that `core::fs_ops` is the single implementation behind both the MCP and REST layers.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-008. DEC-034 was recorded as a decision but obliged no numbered requirement, while Section 7.2 rests every security property on there being one pipeline rather than four.

#### FR-NEW-055 [EARS-E]: The pull quota charge covers only the files written
> WHEN `git.remote_pull` computes the tree it will apply, THE mcp-fs server SHALL charge the summed byte size of only those blobs whose path or content differs from the current volume state, and SHALL charge nothing for a path left untouched.

- **Inputs:** A pull advancing a 200 MB tree in which one 10 KB file changed, with `safety.write_quota_bytes` set to 1 MB.
- **Outputs:** The pull succeeds, charging 10240 bytes.
- **Business Rules:** The charge happens before the first write; an insufficient quota refuses without writing and without advancing the ref (FR-NEW-035). This differs deliberately from clone, which charges the whole tree because every file is new.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-009 and records DEC-036.

#### FR-NEW-056 [EARS-O]: Fetch does not prune remote-tracking refs
> IF a branch present in `refs/remotes/origin/*` no longer exists on the remote, THEN THE mcp-fs server SHALL leave that remote-tracking ref in place and SHALL list it in the response field `refs_stale`.

- **Inputs:** `origin/feature-x` deleted upstream, then `git.remote_fetch`.
- **Outputs:** `refs/remotes/origin/feature-x` still resolves to its previous sha; the response contains `"refs_stale":["refs/remotes/origin/feature-x"]`.
- **Business Rules:** Pruning is a destructive ref operation and is out of scope. No local ref and no file changes (FR-NEW-027).
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-010 and records DEC-037.

#### FR-NEW-057 [EARS-E]: Every remote operation records an audit entry
> WHEN `git.remote_clone`, `git.remote_push`, `git.remote_fetch` or `git.remote_pull` completes, successfully or not, THE mcp-fs server SHALL record one entry through `safety.record_audit` naming the tool, the host, the branch when one is named, and the outcome.

- **Inputs:** A successful push of `main` to `github.ibm.com`, then a push refused as non-fast-forward.
- **Outputs:** Two audit entries, the second recording the refusal.
- **Business Rules:** The entry never contains a token or a userinfo-bearing URL (FR-NEW-042, FR-NEW-043).
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-013. A persisted, user-visible side effect stated only in a non-functional prose bullet, with no requirement and no test behind it.


#### FR-NEW-058 [EARS-U]: The anti-forgery field and its rendering are named
> The mcp-fs server SHALL name the anti-forgery field `csrf_token`, SHALL render it in the `GET /app/tokens` page as `<input type="hidden" name="csrf_token" value="...">`, and SHALL require it as an `application/x-www-form-urlencoded` field of the same name on `POST /app/tokens` and `POST /app/tokens/revoke`.

- **Inputs:** `POST /app/tokens` body `host=github.ibm.com&token=ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH&csrf_token=<value issued by the GET>`.
- **Outputs:** `303 See Other` with `Location: /app/tokens` when the value matches an unconsumed token issued to the same person; `403 application/json {"error":"ERR_FORBIDDEN","detail":"..."}` when it is missing, unknown, already consumed, or issued to a different person.
- **Business Rules:** The value is a UUIDv4 string, held in server memory only, bound to the authenticated person, single use, consumed on first successful match. A request authenticated by a header rather than the `mcpfs_token` cookie is exempt (FR-NEW-048). There is no existing CSRF machinery in the tree to reuse.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A1. The cookie, the routes and the two form fields were all named; this third field was not, so an agent would have had to invent it.

#### FR-NEW-059 [EARS-UB]: The server never emits the session cookie
> The mcp-fs server SHALL NOT emit a `Set-Cookie` header for `mcpfs_token` on any route.

- **Inputs:** `GET /app/tokens`, `POST /app/tokens` and `POST /app/tokens/revoke`, each authenticated by cookie and by header.
- **Outputs:** No response carries `Set-Cookie`.
- **Business Rules:** The cookie is issued by whatever fronts the deployment, which is also where `SameSite=Strict` is configured. FR-NEW-048's cross-origin defence therefore rests on `csrf_token` alone.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A2. FR-NEW-047 said the server never sets the cookie while FR-NEW-048 said it sets `SameSite=Strict`; the two could not both be satisfied, and `Set-Cookie` is directly observable.

#### FR-NEW-060 [EARS-U]: The push response shape is fixed
> The mcp-fs server SHALL return from `git.remote_push` an object with `branch`, a string; `created`, a boolean; `up_to_date`, a boolean; `remote_sha`, a 40 character hex string; and `auth`, a string, all five keys present on every successful call.

- **Inputs:** A push updating `main`; then the same push repeated with no intervening commit.
- **Outputs:** `{"branch":"main","created":false,"up_to_date":false,"remote_sha":"<40 hex>","auth":"github"}`, then the same object with `"up_to_date":true`.
- **Business Rules:** `created` and `up_to_date` are never both true. `remote_sha` is the sha the remote ref holds after the call.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A3. Fetch and `git.auth_status` had their shapes fixed exhaustively; push declared four keys in one requirement and a fifth in another, so `up_to_date` was sometimes present and sometimes absent.

#### FR-NEW-061 [EARS-E]: A successful push advances the remote-tracking ref
> WHEN `git.remote_push` succeeds for a branch, THE mcp-fs server SHALL set `refs/remotes/origin/{branch}` to the pushed sha, creating that ref when it is absent.

- **Inputs:** A push of `main` at sha S into a volume whose `refs/remotes/origin/main` is at an older sha.
- **Outputs:** `git.status` lists `refs/remotes/origin/main` at S. `refs/heads/main` and every volume file are unchanged.
- **Business Rules:** A refused push (FR-NEW-024) advances nothing. An up-to-date push leaves the ref where it is.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A4. `git.status` returns every ref unfiltered (`crates/mcp-fs/src/git/db.rs:261-271` has no prefix filter), so two implementations would produce different visible output after an identical push.

#### FR-NEW-062 [EARS-U]: Fetch uses one explicit refspec and takes no tags
> The mcp-fs server SHALL fetch with the single refspec `+refs/heads/*:refs/remotes/origin/*`, SHALL disable automatic tag following, and SHALL report in `refs_updated` only refs under `refs/remotes/origin/`.

- **Inputs:** A fetch from a remote holding branch `main` and tag `v1`.
- **Outputs:** `refs/remotes/origin/main` updated and listed; `refs/tags/v1` absent locally; `git.tags` unchanged.
- **Business Rules:** libgit2's default fetch follows tags automatically, which is observable through the existing `git.tags` tool (`crates/mcp-fs/src/tools/git.rs:112-113`). Tag synchronisation is out of scope and is recorded in `specs/BACKLOG.md`. `git.remote_pull` inherits this refspec, since its first step is this fetch.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A5.

#### FR-NEW-063 [EARS-O]: Pull targets only the checked-out branch
> IF the `branch` given to `git.remote_pull` is not the branch that `HEAD` currently points at, THEN THE mcp-fs server SHALL reject the call with `ERR_INVALID_ARGUMENT` before fetching, naming both the requested branch and the checked-out branch.

- **Inputs:** A volume whose `HEAD` is `refs/heads/main`, called with `{"mount_id":"proj-a","branch":"feature/x"}`.
- **Outputs:** `ERR_INVALID_ARGUMENT` naming `feature/x` and `main`. No fetch, no ref change, no file change.
- **Business Rules:** The volume is the working tree and has exactly one checked-out branch, so applying another branch's tree to it is never correct. `branch` remains required so the caller states its intent explicitly.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A6. FR-NEW-028 required the volume's files to be updated to the pulled tree while `branch` was a free parameter, so one implementation would overwrite the working tree with a foreign branch's content.

#### FR-NEW-064 [EARS-O]: A failed persistence write leaves no in-memory token
> IF the persistence write of `git.token_set` or of the device-flow poller fails, THEN THE mcp-fs server SHALL remove the in-memory entry for that `(person, host)` before returning the error, restoring the entry that was present before the call when one existed.

- **Inputs:** A seed for `(alice@test.com, github.ibm.com)` with the backing store unreachable, on an empty store; then the same with a prior valid token present.
- **Outputs:** First case: `git.auth_status` lists no entry for that host and a clone fails per EXC-004a. Second case: the prior token is still present and still clones.
- **Business Rules:** `store_token` writes memory first, then persistence (`crates/mcp-fs/src/git/oauth/store.rs:129-141`). Reporting a failure while leaving a live credential in memory makes observable state depend on process lifetime.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A7, and completes FR-NEW-017, which required the call to fail but not what it left behind.

#### FR-NEW-065 [EARS-U]: Ownership of the host map is split between declaration and interpretation
> The mcp-fs server SHALL declare `git.hosts` as a field of `GitConfig` in `crates/mcp-fs/src/config.rs`, SHALL implement its validation and its resolution exactly once in `crates/mcp-fs/src/git/remote.rs` as `git::remote::validate_hosts(&GitConfig) -> Result<()>` and `git::remote::resolve_host(&str) -> Result<Provider>`, and `ServerConfig::validate` SHALL call `validate_hosts` rather than inspecting the entries itself.

- **Inputs:** The source tree after implementation.
- **Outputs:** `config.rs` contains the field declaration and one call to `validate_hosts`, and no other inspection of an entry's value. `remote.rs` is the only file that compares a hostname to the map.
- **Business Rules:** Boot validation still fires from `ServerConfig::validate` (`crates/mcp-fs/src/config.rs:813-819`), preserving the property that misconfiguration fails at boot.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A8. FR-NEW-054 and E2E-NEW-197 required `git.hosts` to be read in exactly one file, which no implementation validating in `config.rs` could satisfy.

#### FR-NEW-066 [EARS-E]: `instance_url` and `host` must agree on `git.auth`
> WHEN `git.auth` is called with a non-null `instance_url`, THE mcp-fs server SHALL parse its hostname, SHALL use that hostname as the host when `host` is omitted, and SHALL reject the call with `ERR_INVALID_ARGUMENT` naming both values when a supplied `host` differs from it.

- **Inputs:** `{"provider":"gitlab","instance_url":"https://gitlab.acme.corp"}`; `{"provider":"gitlab","instance_url":"https://gitlab.acme.corp","host":"gitlab.com"}`; `{"provider":"gitlab","host":"gitlab.acme.corp"}`.
- **Outputs:** The first stores under `(person, gitlab.acme.corp)`; the second is rejected naming both hosts; the third proceeds and keeps `instance_url` null on the stored session.
- **Business Rules:** The derived host must still be declared in `git.hosts` and map to the given provider (EXC-003b). `instance_url` remains on the stored session unchanged (`crates/mcp-fs/src/git/oauth/store.rs:24-32`). The canonical-host default of FR-MOD-002 applies only when both `host` and `instance_url` are absent.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A9. Without this, an enterprise GitLab token obtained through the device flow could be stored under `gitlab.com` and never found again.

#### FR-NEW-067 [EARS-U]: The token screen orders rows by host
> The mcp-fs server SHALL render the token screen rows ordered by host ascending, identically to the ordering FR-NEW-051 fixes for `git.auth_status`.

- **Inputs:** A person holding tokens for `github.ibm.com` and `github.com`.
- **Outputs:** `github.com` renders before `github.ibm.com`.
- **Business Rules:** The screen reads the same per-person enumeration the tool reads (DRIFT-010).
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A10. Row order is what the person sees, and the screen is declared a second adapter over the same operation.


#### FR-NEW-068 [EARS-U]: `git.auth_revoke` takes an optional provider and reports the resolved one
> The mcp-fs server SHALL declare both `provider` and `host` optional on `git.auth_revoke`, SHALL require at least one of the two, SHALL target the provider's canonical public host when only `provider` is given, SHALL target the named host when `host` is given, and SHALL set the response `provider` to the host's value in `git.hosts` when the host is declared and to null when it is not.

- **Inputs:** `{"provider":"github"}`; `{"host":"github.ibm.com"}`; `{"provider":"github","host":"github.ibm.com"}`; `{"host":"git.unknown.test"}`; `{}`.
- **Outputs:** `{"host":"github.com","provider":"github","revoked":true}`; `{"host":"github.ibm.com","provider":"github","revoked":true}`; the same for the third; `{"host":"git.unknown.test","provider":null,"revoked":false}`; `ERR_INVALID_ARGUMENT` naming both parameters for the fifth.
- **Business Rules:** A `host` whose `git.hosts` value disagrees with a supplied `provider` is rejected with `ERR_INVALID_ARGUMENT` naming both, mirroring EXC-003b. An undeclared host is **not** rejected here: revoking a host that can no longer be reached must stay possible, and reports `revoked:false` when nothing was stored. This supersedes the description of `provider` as required in Section 3.1 and in SC-007 step 2.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F1 of round 3. `provider` is required today (`crates/mcp-fs/src/tools/git_auth.rs:118-119`, read at `:122`), yet FR-NEW-051 and E2E-NEW-190 call the tool without it.

#### FR-NEW-069 [EARS-E]: The pull quota charge is the delta
> WHEN `git.remote_pull` computes the tree it will apply, whether by fast-forward or by merge, THE mcp-fs server SHALL charge against the session write quota the summed byte size of only those blobs it will actually write, SHALL charge nothing for a path whose content is unchanged, SHALL perform that charge before the first write, and SHALL refuse the operation without writing anything and without advancing the ref when the quota is insufficient.

- **Inputs:** A 200 MB tree in which one 10 KB file changed, with `safety.write_quota_bytes` of 1 MB; a merged tree whose changed blobs sum to 4 MB, with `safety.write_quota_bytes` of 1 MB.
- **Outputs:** The first succeeds, charging 10240 bytes; the second is refused, no merge commit created and `refs/heads/main` unchanged.
- **Business Rules:** This is the single authority on the basis of the charge. FR-NEW-036 defers to it and FR-NEW-055 is subsumed by it. Clone continues to charge its whole tree (`crates/mcp-fs/src/tools/git.rs:778-782`).
- **Priority:** Must-have
- **Rationale:** Closes audit finding F2 of round 3. FR-NEW-036 required the total tree while FR-NEW-055 required only the delta, and E2E-NEW-133 and E2E-NEW-198 asserted opposite outcomes for the same situation. Both could not pass.

#### FR-NEW-070 [EARS-U]: The credential-absent and credential-expired failures carry fixed codes and prefixes
> The mcp-fs server SHALL report a remote operation against a declared non-anonymous host for which the person holds no token as `ERR_UNAUTHENTICATED` with the message prefix `no token for host`, and a remote operation whose stored token has expired as `ERR_UNAUTHENTICATED` with the message prefix `token expired for host`, each followed by the hostname and an instruction to authenticate with `git.auth` or `git.token_set`.

- **Inputs:** Clone, push, fetch and pull against `github.ibm.com` with no stored token; then the same four with a token expired one hour ago.
- **Outputs:** `ERR_UNAUTHENTICATED: no token for host github.ibm.com ...` for the first four; `ERR_UNAUTHENTICATED: token expired for host github.ibm.com ...` for the second four. No socket is opened in any of the eight.
- **Business Rules:** The two prefixes are distinct so a client can tell absence from expiry, which is the distinction FR-NEW-019 exists to preserve. No new `ERR_*` constant is introduced; `ERR_UNAUTHENTICATED` already exists at `crates/mcp-fs/src/errors.rs:9`.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F3 of round 3. Every other rejection in this specification pins its code; these two named only the message content.

#### FR-NEW-071 [EARS-U]: The four new tool schemas declare their required parameters
> The mcp-fs server SHALL declare `host` and `token` required and `expires_at` optional and nullable on `git.token_set`; `mount_id` and `branch` required on `git.remote_push`; `mount_id` required on `git.remote_fetch`; and `mount_id` and `branch` required with `on_conflict` optional and nullable on `git.remote_pull`.

- **Inputs:** `git.remote_push {"mount_id":"proj-a"}`; `git.remote_pull {"mount_id":"proj-a"}`; `git.token_set {"host":"github.ibm.com","token":"ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH"}`; `git.remote_fetch {"mount_id":"proj-a"}`.
- **Outputs:** `ERR_INVALID_ARGUMENT` naming `branch` for the first two; success for the last two.
- **Business Rules:** `host` stays optional on `git.auth`, `git.auth_status` and `git.auth_revoke` (FR-MOD-002, FR-MOD-003, FR-NEW-068), so those three schema changes remain additive. These lists are what `tool-contract-golden.json` records after regeneration (FR-NEW-045).
- **Priority:** Must-have
- **Rationale:** Closes audit finding F4 of round 3. The frozen contract records a `required` list per tool, so an unstated one is a user-visible artifact left to the implementer.

#### FR-NEW-072 [EARS-E]: Every remote operation emits one tracing span
> WHEN `git.remote_clone`, `git.remote_push`, `git.remote_fetch` or `git.remote_pull` runs, THE mcp-fs server SHALL emit exactly one tracing span named `git.remote`, carrying the fields `operation`, `host`, `provider`, `branch` when the tool names one, `outcome` of `ok` or `error`, and `duration_ms`.

- **Inputs:** A successful push of `main` to `github.ibm.com`; a clone refused for an undeclared host.
- **Outputs:** One span per call, the second with `outcome=error`. Neither span carries a token, an `Authorization` value, or URL userinfo.
- **Business Rules:** The span is emitted for a failure resolved before any network call as well as for a completed operation, so the refusal paths of FR-NEW-007, FR-NEW-049 and FR-NEW-070 are all observable. Content is constrained by FR-NEW-043.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F5 of round 3. The span was required only in a non-functional prose bullet, the same shape as the audit-entry bullet that became FR-NEW-057.

### Modified Requirements

#### FR-MOD-001 [EARS-E]: Provider resolution replaces substring detection (references `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md`)
> WHEN `git.remote_clone` resolves the provider for a URL, THE mcp-fs server SHALL use the `git.hosts` map with exact hostname matching instead of substring inspection of the URL.

- **Original behavior:** `lower.contains("github.com")` then `lower.contains("gitlab")`, else no provider and `auth: "anonymous"` (`crates/mcp-fs/src/tools/git.rs:683-706`).
- **New behavior (EARS):** WHEN resolving a provider, THE mcp-fs server SHALL parse the hostname and match it exactly against `git.hosts`, failing per FR-NEW-007 on a miss.
- **Reason for change:** The substring test is both too narrow, missing `github.ibm.com`, and too broad, matching any URL whose path contains `gitlab`.
- **Business Rules:** As FR-NEW-006 and FR-NEW-007.
- **Priority:** Must-have

#### FR-MOD-002 [EARS-E]: `git.auth` accepts a host (references `FR-624`)
> WHEN `git.auth` is called, THE mcp-fs server SHALL accept an optional `host` and SHALL store the resulting token keyed `(person, host)`.

- **Original behavior:** `git.auth(provider, instance_url?)`, storing keyed `(person, provider)` (`crates/mcp-fs/src/tools/git_auth.rs:76-92`, `:195-207`).
- **New behavior (EARS):** IF `host` is omitted, THEN THE mcp-fs server SHALL use the provider's canonical public host, `github.com` for `github` and `gitlab.com` for `gitlab`.
- **Reason for change:** Token identity is per host; a provider-only flow cannot express which host was authorised.
- **Business Rules:** A `host` mapping to a provider other than the one given is rejected, naming both. A `host` mapping to `generic` or `anonymous` is rejected, because the device flow exists only for `github` and `gitlab`.
- **Priority:** Must-have

#### FR-MOD-003 [EARS-E]: `git.auth_status` reports per host (references `FR-629`)
> WHEN `git.auth_status` is called, THE mcp-fs server SHALL report one entry per stored `(person, host)`, each naming the host, the provider, a validity of `valid` or `expired`, and the expiry, and SHALL NOT include any token value.

- **Original behavior:** One entry per provider (`git_auth.rs:97-115`).
- **New behavior (EARS):** WHEN `provider` or `host` is supplied, THE mcp-fs server SHALL filter the report to matching entries.
- **Reason for change:** One person can hold several tokens per provider, one per host.
- **Business Rules:** An expired token is reported as `expired`, never omitted, which is what FR-NEW-019 preserves.
- **Priority:** Must-have

#### FR-MOD-004 [EARS-E]: `git.auth_revoke` revokes exactly one host (references `FR-629`)
> WHEN `git.auth_revoke` is called, THE mcp-fs server SHALL delete the token for exactly one `(person, host)` pair and SHALL NOT delete any other stored token.

- **Original behavior:** `git.auth_revoke(provider)` deleting the single `(person, provider)` entry (`git_auth.rs:116-127`).
- **New behavior (EARS):** IF `host` is omitted, THEN THE mcp-fs server SHALL target the provider's canonical public host.
- **Reason for change:** With per-host identity, a provider-only revoke would destroy a token for a host the caller never named. `github.com` and `github.ibm.com` are not the same host.
- **Business Rules:** Revoking a host with no stored token succeeds and reports nothing revoked.
- **Priority:** Must-have

---

## 7. Non-Functional Requirements

Only what is new or changed. Inherited and unchanged: tokens encrypted at rest with AES-256-GCM
under `MCPFS_TOKEN_KEY` (`FR-627`); persistence conditional on the key (`FR-628`); git tools
gated by membership, never platform admin (`FR-612`); auth tools gated by authentication alone
(`FR-613`); writes serialized per repository (`FR-609`); misconfiguration fails at boot
(`README.md:140-142`).

### 7.1 Performance

| Requirement | Measure |
|---|---|
| Remote operations run off the request thread | Clone, push, fetch and pull execute on the existing `on_git_thread` path (`crates/mcp-fs/src/tools/git.rs:724`). No request thread blocks on the network or the database. |
| Every remote operation is time-bounded | `git.remote_timeout_secs`, default 120, configurable. Exceeding it aborts and releases the per-repository write lock. |
| Host resolution is constant-time relative to map size | A map lookup, not a scan. Resolution adds no measurable latency to an operation dominated by network transfer. |

### 7.2 Security

| Requirement | Measure |
|---|---|
| No token on any observable surface | FR-NEW-043. Verified by tests asserting the absence of the token substring in responses, traces, audit entries, error messages and HTTP bodies. |
| Transport restricted to HTTPS | FR-NEW-041. `ssh`, `git`, `file` and `http` all rejected. `file://` in particular would expose the server's filesystem. |
| No credentials in URLs | FR-NEW-042. Userinfo-bearing URLs rejected before they can be stored as `origin` or emitted in an audit entry. |
| Token screen authenticated | FR-NEW-037. The existing RS256 identity layer, not a loopback session. |
| Strict per-person isolation | FR-NEW-040. A platform admin gains no access to another person's tokens. |
| Bounded token size | FR-NEW-015. 8192 characters, a denial-of-service bound on `git.token_set`. |
| Encryption unchanged | Tokens continue to be encrypted at rest; only the bearer token is encrypted (`crates/mcp-fs/src/git/oauth/persistence.rs:3`). |

### 7.3 Usability

The failure messages of FR-NEW-007, FR-NEW-018, FR-NEW-021, FR-NEW-024, FR-NEW-029 and
FR-NEW-030 each name the specific remedy: declare the host, re-authenticate, clone first, the
push is not a fast-forward, commit or discard, supply `on_conflict`. A message that names only
the failure and not the remedy does not satisfy these requirements.

### 7.4 Reliability

| Requirement | Measure |
|---|---|
| Pull is atomic | FR-NEW-035. No ref advances over a partially written tree. |
| Boot validation is total | FR-NEW-002 to FR-NEW-004. Every invalid entry is rejected at boot, not at first use. |
| Idempotent remote operations | Push (FR-NEW-025), fetch (EXC-011b) and pull (EXC-012b) all succeed without change when already current. |
| Backend parity | FR-NEW-010 and FR-NEW-012 behave identically on SQLite, PostgreSQL and SQL Server, exercised by the single conformance suite (`crates/mcp-fs/src/storage/conformance.rs`). |

### 7.5 Observability

- **Collector:** unchanged from the project default.
- **What to trace:** each remote operation as a span carrying operation, host, provider, branch, outcome and duration.
- **What to never trace:** token values, the `Authorization` header, userinfo from any URL, and any credential surfaced in a libgit2 error string.
- **Audit:** every remote operation records an entry through `safety.record_audit` naming the operation, host, branch and outcome. The existing clone entry records the URL (`crates/mcp-fs/src/tools/git.rs:794-800`), which stays correct only because FR-NEW-042 prevents a credential-bearing URL from reaching it.

### 7.6 Deployment

No change. The feature adds configuration keys under `git`, not infrastructure. `MCPFS_TOKEN_KEY`
remains the only secret involved, sourced from the environment.

### 7.7 Scalability

Token storage grows as tokens per person multiplied by hosts per person, a small constant.
`oauth_tokens` remains keyed and indexed; `host` is a bounded `TextKey` precisely so SQL Server
can index it.

---

## 8. Data Model

| Entity | Storage | Key | Change |
|---|---|---|---|
| `HostEntry` | In-memory, from YAML | `host` | **New.** `{ host: String, provider: Provider }`. Immutable after boot. |
| `Provider` | In-memory enum | — | **New.** `Github` \| `Gitlab` \| `Generic` \| `Anonymous`. |
| `OAuthSession` | `oauth_tokens` | — | **Modified.** Gains `host`; `provider` becomes a non-key attribute. `access_token`, `scopes`, `expires_at`, `instance_url` unchanged (`crates/mcp-fs/src/git/oauth/store.rs:24-32`). |
| `oauth_tokens` row | Relational | **`(person, host)`** | **Modified.** Was `(person, provider)` (`persistence.rs:42`). `host` is `TextKey`-bounded. All legacy rows dropped (FR-NEW-011). |
| `git_remotes` row | Relational | `(volume_id, name)` | **Unchanged schema, newly written.** `(volume_id, name, url)` (`crates/mcp-fs/src/git/db.rs:69-75`). FR-NEW-020 supplies its first production writer. |

`expires_at` is nullable in effect: an absent expiry means non-expiring (FR-NEW-014). The
existing column is non-null, so the implementation must either relax it or adopt a sentinel; the
requirement is the observable behaviour, and the representation is the implementer's choice
provided `git.auth_status` reports such a token as `valid` indefinitely.

---

## 9. Impact Analysis

### 9.1 Affected Components

| File/Module | Impact | Description |
|---|---|---|
| `crates/mcp-fs/src/git/remote.rs` | **New** | Owns host resolution, credential supply, timeout, URL validation, and clone/push/fetch/pull. The single implementation of the security pipeline (DEC-034). |
| `crates/mcp-fs/src/tools/git.rs` | **Major** | `remote_clone` delegates to `git::remote`; substring detection at `:683-691` deleted; `clone_to_temp` at `:1003-1020` moves; three new tool registrations. Records `origin` (FR-NEW-020). |
| `crates/mcp-fs/src/tools/git_auth.rs` | **Major** | `host` added to three schemas; `git.token_set` added; keying changed at the `store_token` call site (`:195-207`); canonical-host defaulting. |
| `crates/mcp-fs/src/git/oauth/store.rs` | **Major** | `key()` at `:65-67` becomes `(person, host)`; `store_token`, `get_token`, `revoke_token`, `has_valid_token`, `list_ids` signatures change; expiry enforced on the read path used by remote operations. |
| `crates/mcp-fs/src/git/oauth/persistence.rs` | **Major** | Primary key at `:42` becomes `(person, host)`; `host` column added; `upsert`/`delete`/`load` re-keyed; legacy-row drop. |
| `crates/mcp-fs/src/config.rs` | **Moderate** | `GitConfig` at `:447-457` gains `hosts` and `remote_timeout_secs`; boot validation added alongside `validate_store` (`:817`). |
| `crates/mcp-fs/src/git/db.rs` | **None (newly used)** | `add_remote` (`:281`) and `list_remotes` (`:298`) gain their first production callers. No API change. |
| `crates/mcp-fs/src/migrate.rs` | **Moderate** | Carries the new `oauth_tokens` key. |
| `crates/mcp-fs/src/app.rs` | **Moderate** | Registers the token screen route behind the identity layer. |
| `TOOL_CONTRACT.txt`, `tool-contract-golden.json` | **Regenerated** | 3 modified schemas, 4 new tools. |
| `crates/mcp-fs/src/api/dataplane.rs` | **None** | Git has no REST surface. |
| `crates/mcp-fs/src/core/fs_ops.rs` | **None** | Untouched. |

### 9.2 Affected Requirements

| Spec File | Requirement | Impact | Description |
|---|---|---|---|
| `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md` | `FR-624` | Modified | Device authorization gains a host dimension (FR-MOD-002). |
| `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md` | `FR-629` | Modified | Status and revocation become per host (FR-MOD-003, FR-MOD-004). |
| `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md` | `FR-627` | Preserved | Encryption at rest unchanged. |
| `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md` | `FR-628` | Preserved | Conditional persistence unchanged; FR-NEW-017 adds a loud failure when persistence is configured and fails. |
| `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md` | `FR-609` | Preserved and relied upon | Per-repository write serialization is what makes concurrent pushes safe; FR-NEW-044 prevents a hung remote from holding the lock. |
| `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md` | `FR-612`, `FR-613` | Preserved | Membership gating and auth-only gating unchanged; FR-NEW-040 extends the admin-has-no-implicit-access rule to tokens. |
| `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md` | `FR-604`, `FR-605` | Relied upon | Export-before-read and import-after-write are the mechanism push, fetch and pull use. |
| `specs/SPEC-0005_2026-09-18_18-55-00-multi-backend-storage/spec.md` | Backend parity | Impacted | The `oauth_tokens` key change must pass the conformance suite on all three engines. |

No existing requirement is invalidated. One existing **behaviour** is deliberately broken: a
clone from an undeclared host, which previously proceeded anonymously.

### 9.3 Affected Tests

| Test File | Test | Action | Description |
|---|---|---|---|
| `git/oauth/store.rs` | `store_then_get` (:200) | Modify | Re-key to `(person, host)`. |
| `git/oauth/store.rs` | `keying_is_caseless_on_both_parts` (:214) | Modify | Both parts are now person and host. |
| `git/oauth/store.rs` | `store_overwrites_the_same_key_regardless_of_casing` (:225) | Modify | Re-key. |
| `git/oauth/store.rs` | `providers_are_independent` (:234) | Modify | Becomes `hosts_are_independent`; must additionally prove two hosts of the same provider are independent. |
| `git/oauth/store.rs` | `has_valid_token_respects_expiry` (:260) | Modify | Re-key. |
| `git/oauth/store.rs` | `revoke_removes_the_session` (:274) | Modify | Re-key. |
| `git/oauth/store.rs` | `persistent_store_loads_and_writes_through` (:308) | Modify | Re-key. |
| `git/oauth/persistence.rs` | `upsert_then_load` (:231) | Modify | New primary key. |
| `git/oauth/persistence.rs` | `upsert_replaces_the_row_for_the_same_key` (:247) | Modify | New primary key. |
| `git/oauth/persistence.rs` | `one_row_per_person_provider_pair` (:264) | Modify | Becomes `one_row_per_person_host_pair`. |
| `git/oauth/persistence.rs` | `delete_removes_one_row_and_is_idempotent` (:279) | Modify | New primary key. |
| `git/oauth/persistence.rs` | `token_is_not_stored_in_clear` (:291) | Modify | New primary key; assertion preserved. |
| `git/oauth/persistence.rs` | `rows_encrypted_with_another_key_are_skipped_not_fatal` (:306) | Modify | New primary key. |
| `git/oauth/persistence.rs` | `corrupt_blobs_and_bad_timestamps_are_skipped` (:329) | Modify | New primary key. |
| `git/oauth/persistence.rs` | `scopes_round_trip_including_empty` (:349) | Modify | New primary key. |
| `tools/git_auth.rs` | `git_auth_schema_matches_the_contract` (:371) | Modify | Schema gains `host`. |
| `tools/git_auth.rs` | `git_auth_status_schema_has_no_required_parameter` (:390) | Modify | Schema gains `host`; still no required parameter. |
| `tools/git_auth.rs` | `pending_then_success_stores_the_token` (:421) | Modify | Assert storage keyed by host. |
| `tools/git_auth.rs` | `gitlab_keeps_the_instance_url_on_the_stored_session` (:454) | Modify | Interaction between `instance_url` and the new `host`. |
| `tools/git_auth.rs` | `auth_status_reports_one_provider` (:487) | Modify | Reports per host. |
| `tools/git_auth.rs` | `auth_status_reports_all_providers_when_none_is_given` (:507) | Modify | Reports all hosts. |
| `tools/git_auth.rs` | `an_expired_token_reports_as_unauthenticated` (:526) | Modify | Must now report `expired`, distinct from absent (FR-NEW-019). |
| `tools/git_auth.rs` | `revoke_clears_the_token_and_is_idempotent` (:545) | Modify | Per host; must prove no other host is affected. |
| `tools/git_auth.rs` | `tokens_are_per_person` (:559) | Modify | Re-key; extended by E2E-NEW-162. |
| `tools/git.rs` | `git_remote_clone_schema_matches_the_contract` | Modify | Contract regenerated. |
| `tools/contract_golden.rs` | `tool_contract_golden_is_current` (:129) | Modify | Regenerated golden. |

**26 tests modified, none removed.** Nothing is removed because no behaviour is deleted: every
affected test survives with its key or schema updated.

**Coverage gap in the existing suite, worth stating plainly:** there is no current test of
provider detection at all. The defect at `git.rs:683-691` is untested in both directions, which
is why it survived. E2E-NEW-011 and E2E-NEW-012 close that gap.

### 9.4 Affected Documentation

| Document | Section | Action | Description |
|---|---|---|---|
| `AGENTS.md` | Overview, tool counts | Update | Tool count rises by four; the `git.*` family description gains remote sync. |
| `AGENTS.md` | Behaviour worth knowing | Update | Add per-host token identity and the undeclared-host break. |
| `README.md` | Configuration | Update | Document `git.hosts` and `git.remote_timeout_secs`. |
| `README.md` | Git section | Update | Document push, fetch, pull and the token screen. |
| `.agent_docs/git.md` | All | Update | Host map, per-host tokens, remote pipeline, merge strategy. |
| `.agent_docs/config.md` | Git section | Update | New keys, their validation, and the failure messages. |
| `.agent_docs/tools.md` | Tool reference | Update | Four new tools, three modified. |
| `.agent_docs/lineage.md` | Design decisions | Update | Record the undeclared-host break and the drop-on-upgrade decision. |
| `.agent_docs/testing.md` | Suites | Update | Note the backend parity requirement for the new key. |
| `TOOL_CONTRACT.txt` | Git families | Regenerate | Never hand-edited. |
| `specs/BACKLOG.md` | — | Append | Nine deferred items (Section 15). |

### 9.5 Dependencies & Risks

**One new direct dependency.** `git2` 0.20.4 already provides merge (`src/merge.rs:133`,
`src/repo.rs:2177`). The `url` crate, which FR-NEW-006 needs for hostname parsing, is present in
`Cargo.lock` as a transitive dependency only: it appears in neither `Cargo.toml`
`[workspace.dependencies]` nor `crates/mcp-fs/Cargo.toml` `[dependencies]`, and no `url::`
reference exists in `crates/mcp-fs/src`. It must be promoted to a direct dependency. Tracked as
DRIFT-003.

**Removable dependencies:** none.

**Breaking changes for consumers:**
1. A clone from an undeclared host now fails where it previously succeeded anonymously. Deliberate (DEC-010). Mitigation: declare `anonymous` hosts explicitly.
2. Every stored token is dropped on upgrade. Deliberate (DEC-005). Mitigation: operators announce a one-time re-authentication.
3. Three tool schemas change. Additive only: `host` is optional everywhere, so existing calls keep working via canonical-host defaulting.

**Migration:** one schema change on `oauth_tokens`, destructive by design, on three backends.

**Rollback strategy:** reverting the code does not restore dropped tokens. A rollback therefore
costs a second re-authentication. Operators are told this before upgrading, and the upgrade is
not reversible in the data sense — only in the code sense.

**Principal risk:** the merge path is the largest new surface and the least covered by existing
machinery. It is mitigated by delegating conflict resolution entirely to libgit2's
`file_favor`, by forbidding conflict markers (FR-NEW-032), and by atomicity (FR-NEW-035).

---

## 10. Documentation Requirements

### 10.1 README.md
Document `git.hosts` with a worked example covering all four providers; `git.remote_timeout_secs`;
the four new tools; the token screen route; and, prominently, the two breaking changes with their
remedies.

### 10.2 AGENTS.md and .agent_docs/
`AGENTS.md`: update the tool count, the git family description, and "Behaviour worth knowing"
with per-host token identity and the undeclared-host break. `.agent_docs/git.md` gains the host
map, the remote pipeline and the merge strategy. `.agent_docs/config.md` gains the new keys and
their boot-validation messages. `.agent_docs/tools.md` gains the four tools.
`.agent_docs/lineage.md` records the two deliberate breaks and why.

### 10.3 docs/*
An operator runbook for the upgrade: what is dropped, how to tell people, how to declare hosts,
and how to verify with `git.auth_status` afterwards.

---

## 11. Traceability Matrix

Every identifier is spelled in full so this table is machine-checkable.

| Scenario | Functional Req | E2E Tests (Happy) | E2E Tests (Failure) | E2E Tests (Edge) |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005 | E2E-NEW-001, E2E-NEW-002 | E2E-NEW-003, E2E-NEW-004, E2E-NEW-005, E2E-NEW-006, E2E-NEW-007, E2E-NEW-008, E2E-NEW-009 | E2E-NEW-010, E2E-NEW-013, E2E-NEW-014, E2E-NEW-015, E2E-NEW-016, E2E-NEW-017 |
| SC-002 | FR-NEW-013, FR-NEW-014, FR-NEW-015, FR-NEW-016, FR-NEW-017 | E2E-NEW-020, E2E-NEW-021, E2E-NEW-022 | E2E-NEW-023, E2E-NEW-024, E2E-NEW-025, E2E-NEW-026, E2E-NEW-027, E2E-NEW-028, E2E-NEW-029, E2E-NEW-030 | E2E-NEW-031, E2E-NEW-032, E2E-NEW-033, E2E-NEW-034, E2E-NEW-035 |
| SC-003 | FR-MOD-002 | E2E-NEW-044, E2E-NEW-045 | E2E-NEW-046, E2E-NEW-047, E2E-NEW-048 | E2E-NEW-049, E2E-NEW-050 |
| SC-004 | FR-NEW-006, FR-NEW-008, FR-NEW-020, FR-MOD-001 | E2E-NEW-011, E2E-NEW-018, E2E-NEW-051 | E2E-NEW-053, E2E-NEW-054, E2E-NEW-055 | E2E-NEW-052, E2E-NEW-056, E2E-NEW-057 |
| SC-005 | FR-NEW-007, FR-MOD-001 | E2E-NEW-019 | E2E-NEW-012, E2E-NEW-058, E2E-NEW-059, E2E-NEW-060 | E2E-NEW-061, E2E-NEW-062 |
| SC-006 | FR-NEW-037, FR-NEW-038, FR-NEW-039, FR-NEW-040 | E2E-NEW-100, E2E-NEW-101, E2E-NEW-102, E2E-NEW-103 | E2E-NEW-104, E2E-NEW-105, E2E-NEW-106, E2E-NEW-107, E2E-NEW-108, E2E-NEW-109, E2E-NEW-110 | E2E-NEW-111, E2E-NEW-112, E2E-NEW-113 |
| SC-007 | FR-MOD-003, FR-MOD-004 | E2E-NEW-063, E2E-NEW-064 | E2E-NEW-065, E2E-NEW-066, E2E-NEW-067 | E2E-NEW-068, E2E-NEW-069, E2E-NEW-070 |
| SC-008 | FR-NEW-009, FR-NEW-010, FR-NEW-011, FR-NEW-012 | E2E-NEW-036, E2E-NEW-037, E2E-NEW-038 | E2E-NEW-039, E2E-NEW-042, E2E-NEW-163 | E2E-NEW-040, E2E-NEW-041, E2E-NEW-043 |
| SC-009 | FR-NEW-021, FR-NEW-022, FR-NEW-023, FR-NEW-025 | E2E-NEW-071, E2E-NEW-072, E2E-NEW-073, E2E-NEW-074 | E2E-NEW-075, E2E-NEW-078, E2E-NEW-079, E2E-NEW-080, E2E-NEW-081 | E2E-NEW-082, E2E-NEW-083, E2E-NEW-084 |
| SC-010 | FR-NEW-024 | E2E-NEW-071 | E2E-NEW-076, E2E-NEW-077 | E2E-NEW-085 |
| SC-011 | FR-NEW-026, FR-NEW-027 | E2E-NEW-086, E2E-NEW-087, E2E-NEW-088 | E2E-NEW-089, E2E-NEW-090, E2E-NEW-091 | E2E-NEW-092, E2E-NEW-093, E2E-NEW-094 |
| SC-012 | FR-NEW-028, FR-NEW-029, FR-NEW-035, FR-NEW-036 | E2E-NEW-114, E2E-NEW-115, E2E-NEW-116 | E2E-NEW-120, E2E-NEW-121, E2E-NEW-122, E2E-NEW-124 | E2E-NEW-123, E2E-NEW-131, E2E-NEW-132, E2E-NEW-133 |
| SC-013 | FR-NEW-030 | E2E-NEW-117 | E2E-NEW-125, E2E-NEW-126, E2E-NEW-127 | E2E-NEW-134 |
| SC-014 | FR-NEW-018, FR-NEW-019 | E2E-NEW-095 | E2E-NEW-096, E2E-NEW-097, E2E-NEW-098 | E2E-NEW-099, E2E-NEW-119 |
| SC-015 | FR-NEW-031, FR-NEW-032, FR-NEW-033, FR-NEW-034 | E2E-NEW-118, E2E-NEW-128, E2E-NEW-129 | E2E-NEW-135, E2E-NEW-136, E2E-NEW-137, E2E-NEW-138 | E2E-NEW-130, E2E-NEW-139, E2E-NEW-140, E2E-NEW-141 |
| Cross-cutting: security | FR-NEW-041, FR-NEW-042, FR-NEW-043, FR-NEW-044 | E2E-NEW-142 | E2E-NEW-143, E2E-NEW-144, E2E-NEW-145, E2E-NEW-146, E2E-NEW-147, E2E-NEW-148, E2E-NEW-149, E2E-NEW-150, E2E-NEW-151, E2E-NEW-152, E2E-NEW-153 | E2E-NEW-154, E2E-NEW-155, E2E-NEW-156, E2E-NEW-157 |
| Cross-cutting: contract | FR-NEW-045 | E2E-NEW-158, E2E-NEW-159, E2E-NEW-160 | E2E-NEW-161 | — |
| Cross-cutting: identity | FR-NEW-009, FR-NEW-040 | E2E-NEW-162 | E2E-NEW-107, E2E-NEW-108 | E2E-NEW-163 |
| SC-006 (audit amendments) | FR-NEW-046, FR-NEW-047, FR-NEW-048 | E2E-NEW-164, E2E-NEW-165, E2E-NEW-166, E2E-NEW-169, E2E-NEW-170, E2E-NEW-171, E2E-NEW-177 | E2E-NEW-167, E2E-NEW-172, E2E-NEW-173, E2E-NEW-174, E2E-NEW-176, E2E-NEW-178 | E2E-NEW-168, E2E-NEW-175, E2E-NEW-179, E2E-NEW-180 |
| Cross-cutting: error codes | FR-NEW-049 | — | E2E-NEW-181, E2E-NEW-182, E2E-NEW-183 | E2E-NEW-184 |
| SC-011 (audit amendments) | FR-NEW-050, FR-NEW-056 | E2E-NEW-185 | E2E-NEW-187 | E2E-NEW-186, E2E-NEW-199 |
| SC-007 (audit amendments) | FR-NEW-051 | E2E-NEW-188 | E2E-NEW-190 | E2E-NEW-189 |
| SC-002 (audit amendments) | FR-NEW-052 | E2E-NEW-191 | E2E-NEW-192 | E2E-NEW-193 |
| Cross-cutting: auth field | FR-NEW-053 | E2E-NEW-194 | E2E-NEW-196 | E2E-NEW-195 |
| Cross-cutting: pipeline | FR-NEW-054 | E2E-NEW-197 | — | — |
| SC-012 (audit amendments) | FR-NEW-055 | E2E-NEW-198 | — | — |
| Cross-cutting: audit trail | FR-NEW-057 | E2E-NEW-200 | E2E-NEW-201, E2E-NEW-202 | — |
| SC-006 (round 2) | FR-NEW-058, FR-NEW-059, FR-NEW-067 | E2E-NEW-203, E2E-NEW-204, E2E-NEW-230 | E2E-NEW-205, E2E-NEW-206, E2E-NEW-207, E2E-NEW-232 | E2E-NEW-208, E2E-NEW-231 |
| SC-009 (round 2) | FR-NEW-060, FR-NEW-061 | E2E-NEW-209, E2E-NEW-212, E2E-NEW-213 | E2E-NEW-210, E2E-NEW-214 | E2E-NEW-211 |
| SC-011 (round 2) | FR-NEW-062 | E2E-NEW-215 | E2E-NEW-216 | E2E-NEW-217 |
| SC-012 (round 2) | FR-NEW-063 | E2E-NEW-219 | E2E-NEW-218 | E2E-NEW-220 |
| SC-002 (round 2) | FR-NEW-064 | — | E2E-NEW-221, E2E-NEW-222 | E2E-NEW-223 |
| SC-001 (round 2) | FR-NEW-065 | E2E-NEW-224, E2E-NEW-225 | — | E2E-NEW-226 |
| SC-003 (round 2) | FR-NEW-066 | E2E-NEW-227 | E2E-NEW-228 | E2E-NEW-229 |
| SC-007 (round 3) | FR-NEW-068 | E2E-NEW-233, E2E-NEW-234 | E2E-NEW-235, E2E-NEW-236 | E2E-NEW-237 |
| SC-012 (round 3) | FR-NEW-069 | E2E-NEW-238 | E2E-NEW-239 | E2E-NEW-240 |
| SC-014 (round 3) | FR-NEW-070 | — | E2E-NEW-241, E2E-NEW-242 | E2E-NEW-243 |
| Cross-cutting: schemas | FR-NEW-071 | E2E-NEW-246 | E2E-NEW-244, E2E-NEW-245 | — |
| Cross-cutting: tracing | FR-NEW-072 | E2E-NEW-247 | — | — |

Every scenario carries at least one happy, one failure and one edge test. All 72 `FR-NEW-*` and
all 4 `FR-MOD-*` appear at least once. The rows added for the audit amendments extend existing
scenarios rather than introducing new ones, so the per-scenario happy/failure/edge guarantee is
satisfied by the original row for each scenario taken together with its amendment row.

### 11.1 Per-requirement test coverage

Each requirement carries at least three tests: one happy, one failure, one edge.

| Requirement | Tests | Count |
|---|---|---|
| FR-NEW-001 | E2E-NEW-001, 002, 003, 013, 014 | 5 |
| FR-NEW-002 | E2E-NEW-003, 004, 005 | 3 |
| FR-NEW-003 | E2E-NEW-001, 006, 002 | 3 |
| FR-NEW-004 | E2E-NEW-007, 008, 009, 010 | 4 |
| FR-NEW-005 | E2E-NEW-017, 111, 001 | 3 |
| FR-NEW-006 | E2E-NEW-001, 011, 012, 015, 016, 055, 057, 061, 062 | 9 |
| FR-NEW-007 | E2E-NEW-012, 019, 050, 053, 058, 059, 060, 090 | 8 |
| FR-NEW-008 | E2E-NEW-018, 028, 083 | 3 |
| FR-NEW-009 | E2E-NEW-031, 032, 033, 036, 043, 082, 162, 163 | 8 |
| FR-NEW-010 | E2E-NEW-036, 038, 040 | 3 |
| FR-NEW-011 | E2E-NEW-037, 039, 040 | 3 |
| FR-NEW-012 | E2E-NEW-038, 041, 042 | 3 |
| FR-NEW-013 | E2E-NEW-020, 021, 022, 023, 029, 031 | 6 |
| FR-NEW-014 | E2E-NEW-035, 020, 099 | 3 |
| FR-NEW-015 | E2E-NEW-024, 025, 026, 034 | 4 |
| FR-NEW-016 | E2E-NEW-027, 028, 020 | 3 |
| FR-NEW-017 | E2E-NEW-030, 035, 020 | 3 |
| FR-NEW-018 | E2E-NEW-095, 096, 098, 099, 119 | 5 |
| FR-NEW-019 | E2E-NEW-054, 070, 097 | 3 |
| FR-NEW-020 | E2E-NEW-051, 052, 075 | 3 |
| FR-NEW-021 | E2E-NEW-075, 080, 089, 124 | 4 |
| FR-NEW-022 | E2E-NEW-071, 073, 078, 079, 081, 082, 084, 130 | 8 |
| FR-NEW-023 | E2E-NEW-072, 071, 078 | 3 |
| FR-NEW-024 | E2E-NEW-076, 077, 085 | 3 |
| FR-NEW-025 | E2E-NEW-074, 071, 084 | 3 |
| FR-NEW-026 | E2E-NEW-086, 087, 091, 092, 093 | 5 |
| FR-NEW-027 | E2E-NEW-088, 094, 086 | 3 |
| FR-NEW-028 | E2E-NEW-114, 115, 116, 131, 134 | 5 |
| FR-NEW-029 | E2E-NEW-120, 121, 122, 123, 138 | 5 |
| FR-NEW-030 | E2E-NEW-117, 125, 126, 127 | 4 |
| FR-NEW-031 | E2E-NEW-118, 123, 128, 129, 130, 134, 139 | 7 |
| FR-NEW-032 | E2E-NEW-136, 118, 128 | 3 |
| FR-NEW-033 | E2E-NEW-135, 118, 128 | 3 |
| FR-NEW-034 | E2E-NEW-140, 118, 139 | 3 |
| FR-NEW-035 | E2E-NEW-132, 137, 114 | 3 |
| FR-NEW-036 | E2E-NEW-133, 141, 114 | 3 |
| FR-NEW-037 | E2E-NEW-100, 105, 106 | 3 |
| FR-NEW-038 | E2E-NEW-100, 103, 104, 109 | 4 |
| FR-NEW-039 | E2E-NEW-101, 102, 110, 112, 113 | 5 |
| FR-NEW-040 | E2E-NEW-107, 108, 162 | 3 |
| FR-NEW-041 | E2E-NEW-142, 143, 144, 145, 146, 147 | 6 |
| FR-NEW-042 | E2E-NEW-148, 149, 150 | 3 |
| FR-NEW-043 | E2E-NEW-023, 056, 065, 104, 151, 152, 153, 154 | 8 |
| FR-NEW-044 | E2E-NEW-155, 156, 157 | 3 |
| FR-NEW-045 | E2E-NEW-158, 159, 160, 161 | 4 |
| FR-NEW-046 | E2E-NEW-164, 165, 166, 167, 168 | 5 |
| FR-NEW-047 | E2E-NEW-169, 170, 171, 172, 173, 174, 175 | 7 |
| FR-NEW-048 | E2E-NEW-176, 177, 178, 179, 180 | 5 |
| FR-NEW-049 | E2E-NEW-181, 182, 183, 184 | 4 |
| FR-NEW-050 | E2E-NEW-185, 186, 187 | 3 |
| FR-NEW-051 | E2E-NEW-188, 189, 190 | 3 |
| FR-NEW-052 | E2E-NEW-191, 192, 193 | 3 |
| FR-NEW-053 | E2E-NEW-194, 195, 196 | 3 |
| FR-NEW-054 | E2E-NEW-197, 151, 152 | 3 |
| FR-NEW-055 | E2E-NEW-198, 133, 141 | 3 |
| FR-NEW-056 | E2E-NEW-199, 093, 185 | 3 |
| FR-NEW-057 | E2E-NEW-200, 201, 202 | 3 |
| FR-NEW-058 | E2E-NEW-203, 204, 205 | 3 |
| FR-NEW-059 | E2E-NEW-206, 207, 208 | 3 |
| FR-NEW-060 | E2E-NEW-209, 210, 211 | 3 |
| FR-NEW-061 | E2E-NEW-212, 213, 214 | 3 |
| FR-NEW-062 | E2E-NEW-215, 216, 217 | 3 |
| FR-NEW-063 | E2E-NEW-218, 219, 220 | 3 |
| FR-NEW-064 | E2E-NEW-221, 222, 223 | 3 |
| FR-NEW-065 | E2E-NEW-224, 225, 226 | 3 |
| FR-NEW-066 | E2E-NEW-227, 228, 229 | 3 |
| FR-NEW-067 | E2E-NEW-230, 231, 232 | 3 |
| FR-NEW-068 | E2E-NEW-233, 234, 235, 236, 237 | 5 |
| FR-NEW-069 | E2E-NEW-238, 239, 240 | 3 |
| FR-NEW-070 | E2E-NEW-241, 242, 243 | 3 |
| FR-NEW-071 | E2E-NEW-244, 245, 246 | 3 |
| FR-NEW-072 | E2E-NEW-247, 151, 153 | 3 |
| FR-MOD-001 | E2E-NEW-011, 012, 019 | 3 |
| FR-MOD-002 | E2E-NEW-044, 045, 046, 047, 048, 049, 050, 161 | 8 |
| FR-MOD-003 | E2E-NEW-063, 064, 065, 069, 070, 161 | 6 |
| FR-MOD-004 | E2E-NEW-066, 067, 068, 161 | 4 |

## 12. End-to-End Test Suite

> E2E tests are the primary contract for implementation. The implementation is correct when all
> of these pass.

Unit and integration tests live alongside the code they exercise, in `#[cfg(test)]` modules
(`crates/mcp-fs/src/tools/git.rs`, `git_auth.rs`, `git/oauth/store.rs`, `git/oauth/persistence.rs`).
A separate shell-driven functional suite also exists at `tests/functional/`, with `run_all.sh`
and 15 scenario scripts including `tests/functional/scenarios/09_git.sh`
(`.agent_docs/testing.md:171-189`). No current functional scenario exercises `git.remote_clone`
or any `git.auth*` tool. The core journeys of this specification, E2E-NEW-020, E2E-NEW-022,
E2E-NEW-071, E2E-NEW-114 and E2E-NEW-118, SHALL additionally be covered by a functional
scenario script, so the remote surface is exercised end to end and not only in-process.

Shared fixtures used throughout: person `alice@test.com`; second person `bob@test.com`; mount
`proj-a`; tokens `ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH` and
`glpat_1111222233334444555566667777`; hosts `github.com`, `github.ibm.com`, `gitlab.acme.corp`,
`git.acme.internal`, `public.example.org`, `git.unknown.test`. The reference map, unless a test
says otherwise:

```yaml
git:
  hosts:
    github.com: github
    github.ibm.com: github
    gitlab.acme.corp: gitlab
    git.acme.internal: generic
    public.example.org: anonymous
```

### 12.1 Test Summary

| Test ID | Action | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-NEW-001…010, 013…017 | New | Feature / Error / Edge | SC-001 | FR-NEW-001…005 | Critical |
| E2E-NEW-011, 012, 018, 019 | New | Security / Error | SC-004, SC-005 | FR-NEW-006, 007, 008, FR-MOD-001 | Critical |
| E2E-NEW-020…035 | New | Feature / Error / Security | SC-002 | FR-NEW-013…017 | Critical |
| E2E-NEW-036…043 | New | Data Integrity / State Transition | SC-008 | FR-NEW-009, 010, 011, 012 | Critical |
| E2E-NEW-044…050 | New | Feature / Error | SC-003 | FR-MOD-002 | High |
| E2E-NEW-051…057 | New | Core Journey / Side Effect | SC-004 | FR-NEW-006, 008, 020 | Critical |
| E2E-NEW-058…062 | New | Error / Edge | SC-005 | FR-NEW-007 | Critical |
| E2E-NEW-063…070 | New | Feature / Error | SC-007 | FR-MOD-003, FR-MOD-004 | High |
| E2E-NEW-071…085 | New | Core Journey / Error / State Transition | SC-009, SC-010 | FR-NEW-021…025 | Critical |
| E2E-NEW-086…094 | New | Feature / Side Effect | SC-011 | FR-NEW-026, 027 | Critical |
| E2E-NEW-095…099 | New | Error / Security | SC-014 | FR-NEW-018, 019 | Critical |
| E2E-NEW-100…113 | New | Security / Feature | SC-006 | FR-NEW-037…040 | Critical |
| E2E-NEW-114…141 | New | Core Journey / State Transition / Data Integrity | SC-012, SC-013, SC-015 | FR-NEW-028…036 | Critical |
| E2E-NEW-142…157 | New | Security / Performance | Cross-cutting | FR-NEW-041…044 | Critical |
| E2E-NEW-158…161 | New | Integration | Cross-cutting | FR-NEW-045 | Critical |
| E2E-NEW-162, 163 | New | Security / Data Integrity | SC-006, SC-008 | FR-NEW-009, 040 | Critical |
| E2E-MOD-001…026 | Modified | Regression | All | All | Critical |

**Coverage Statistics.** Counted from the `**Category:**` line of every specified test, not
estimated. Each test carries exactly one category, so the groupings below are partitions of that
set and reconcile to the total.

| Category | Count |
|---|---|
| Error | 75 |
| Edge | 57 |
| Feature | 36 |
| Security | 36 |
| Side Effect | 10 |
| Core Journey | 7 |
| State Transition | 7 |
| Data Integrity | 7 |
| Performance | 6 |
| Integration | 6 |
| **Total new** | **247** |

- **Happy path** (Feature + Core Journey + Integration): **49**
- **Failure/error** (Error + Security): **111**
- **Edge cases:** 57
- Side effects 10, state transitions 7, data integrity 7, performance 6
- **Modified: 26. Removed: 0.**
- **Happy : Failure = 49 : 111 = 1 : 2.27** — failure tests outnumber happy-path tests.

### 12.2 New Test Specifications

#### Group A — host-to-provider map (SC-001)

**E2E-NEW-001** — Valid map boots and resolves every provider class
- **Category:** Feature · **Scenario:** SC-001 · **Requirements:** FR-NEW-001, FR-NEW-006 · **Priority:** Critical
- Given the reference map above
- When the server starts and each of the five hosts is resolved
- Then the server boots successfully
- And `github.com` resolves to `github`, `github.ibm.com` to `github`, `gitlab.acme.corp` to `gitlab`, `git.acme.internal` to `generic`, `public.example.org` to `anonymous`

**E2E-NEW-002** — Two hosts may share one provider
- **Category:** Feature · **Scenario:** SC-001 · **Requirements:** FR-NEW-001 · **Priority:** Critical
- Given `github.com: github` and `github.ibm.com: github`
- When the server starts
- Then it boots and both hosts resolve to `github` independently

**E2E-NEW-003** — Unknown provider value fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-002 · **Priority:** Critical
- Given `git.hosts` containing `github.ibm.com: githib`
- When the server starts
- Then startup fails
- And the error message contains `github.ibm.com` and lists `github`, `gitlab`, `generic`, `anonymous`

**E2E-NEW-004** — Uppercase provider value fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-002 · **Priority:** High
- Given `github.com: GitHub`
- When the server starts
- Then startup fails naming `github.com`, because provider values are lowercase-exact

**E2E-NEW-005** — Empty provider value fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-002 · **Priority:** High
- Given `github.com: ""`
- When the server starts
- Then startup fails naming `github.com`

**E2E-NEW-006** — Duplicate host fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-003 · **Priority:** Critical
- Given `github.com: github` appearing twice, the second time as `github.com: anonymous`
- When the server starts
- Then startup fails and the error names `github.com`
- And the server does not silently choose either entry

**E2E-NEW-007** — Host with a scheme fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-004 · **Priority:** Critical
- Given `https://github.com: github`
- When the server starts
- Then startup fails naming `https://github.com`

**E2E-NEW-008** — Host with a path fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-004 · **Priority:** Critical
- Given `github.com/org: github`
- When the server starts
- Then startup fails naming `github.com/org`

**E2E-NEW-009** — Host with a port fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-004 · **Priority:** Critical
- Given `github.com:443: github`
- When the server starts
- Then startup fails naming the key

**E2E-NEW-010** — Wildcard host fails boot
- **Category:** Edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-004 · **Priority:** Critical
- Given `*.acme.corp: gitlab`
- When the server starts
- Then startup fails naming `*.acme.corp`
- And the failure message does not suggest wildcards are supported

**E2E-NEW-011** — Regression: an enterprise GitHub host resolves to github
- **Category:** Security · **Scenario:** SC-004 · **Requirements:** FR-NEW-006, FR-MOD-001 · **Priority:** Critical
- Given `github.ibm.com: github` and a token stored for `(alice@test.com, github.ibm.com)`
- When `git.remote_clone` is called with `https://github.ibm.com/org/repo.git`
- Then the resolved provider is `github`
- And the stored token is supplied as the credential
- And the response reports `"auth": "github"`, not `"anonymous"`
- *This is the too-narrow half of the defect at `crates/mcp-fs/src/tools/git.rs:683-691`.*

**E2E-NEW-012** — Regression: a path containing "gitlab" does not resolve to gitlab
- **Category:** Security · **Scenario:** SC-005 · **Requirements:** FR-NEW-006, FR-NEW-007, FR-MOD-001 · **Priority:** Critical
- Given the reference map, which does not declare `exemple.test`
- And a token stored for `(alice@test.com, gitlab.acme.corp)`
- When `git.remote_clone` is called with `https://exemple.test/mirrors/mygitlab-mirror.git`
- Then the call fails as an undeclared host, naming `exemple.test`
- And no token is looked up, and the `gitlab.acme.corp` token is never supplied to any remote
- *This is the too-broad half of the same defect.*

**E2E-NEW-013** — Empty map boots
- **Category:** Edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-001 · **Priority:** High
- Given `git.hosts: {}`
- When the server starts
- Then it boots successfully
- And a subsequent clone of `https://github.com/o/r.git` fails as an undeclared host

**E2E-NEW-014** — Absent map section boots
- **Category:** Edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-001 · **Priority:** High
- Given a `git` config with no `hosts` key
- When the server starts
- Then it boots, and behaves as E2E-NEW-013

**E2E-NEW-015** — Host matching is case-insensitive
- **Category:** Edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-006 · **Priority:** High
- Given `github.ibm.com: github`
- When a clone URL of `https://GitHub.IBM.COM/org/repo.git` is resolved
- Then the provider is `github`

**E2E-NEW-016** — Near-miss hostnames do not match
- **Category:** Edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-006 · **Priority:** Critical
- Given only `github.com: github`
- When each of `ithub.com`, `github.com.evil.test`, `notgithub.com`, `github.co` is resolved
- Then every one fails as an undeclared host
- And none resolves to `github`

**E2E-NEW-017** — The map cannot be mutated at runtime
- **Category:** Security · **Scenario:** SC-001 · **Requirements:** FR-NEW-005 · **Priority:** Critical
- Given a running server with the reference map
- When the registered tool list and HTTP route table are enumerated
- Then no tool and no route accepts input that changes a host-to-provider entry

**E2E-NEW-018** — A declared anonymous host clones without a credential
- **Category:** Feature · **Scenario:** SC-004 · **Requirements:** FR-NEW-008 · **Priority:** Critical
- Given `public.example.org: anonymous` and no token stored for that host
- When `git.remote_clone` is called with `https://public.example.org/o/r.git`
- Then the clone succeeds
- And the response reports `"auth": "anonymous"`
- And no token lookup was performed for `(alice@test.com, public.example.org)`

**E2E-NEW-019** — An undeclared host fails before any network activity
- **Category:** Error · **Scenario:** SC-005 · **Requirements:** FR-NEW-007 · **Priority:** Critical
- Given the reference map and a listener recording outbound connections
- When `git.remote_clone` is called with `https://git.unknown.test/o/r.git`
- Then the call fails with `ERR_INVALID_ARGUMENT` naming `git.unknown.test`
- And the message states the host must be declared in `git.hosts`
- And zero outbound connections were attempted

#### Group C — `git.token_set` (SC-002)

**E2E-NEW-020** — Seeding stores a token for a declared host
- **Category:** Core Journey · **Scenario:** SC-002 · **Requirements:** FR-NEW-013 · **Priority:** Critical
- Given `alice@test.com` authenticated and `github.ibm.com: github`
- When `git.token_set` is called with `{"host": "github.ibm.com", "token": "ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH"}`
- Then the response is `{"host": "github.ibm.com", "provider": "github", "stored": true, "persistent": true}`
- And `git.auth_status` lists `github.ibm.com` with provider `github` and validity `valid`

**E2E-NEW-021** — Seeding derives the provider from the map, not the caller
- **Category:** Feature · **Scenario:** SC-002 · **Requirements:** FR-NEW-013 · **Priority:** High
- Given `git.acme.internal: generic`
- When `git.token_set` is called for that host
- Then the stored session's provider is `generic`
- And the tool schema exposes no `provider` parameter

**E2E-NEW-022** — A seeded token authenticates a clone end to end
- **Category:** Core Journey · **Scenario:** SC-002 · **Requirements:** FR-NEW-013, FR-NEW-006 · **Priority:** Critical
- Given a token seeded for `(alice@test.com, github.ibm.com)`
- When `git.remote_clone` is called with `https://github.ibm.com/org/repo.git`
- Then the credential supplied to the remote is `oauth2:ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH`
- And the clone succeeds with `"auth": "github"`

**E2E-NEW-023** — The response never echoes the token
- **Category:** Security · **Scenario:** SC-002 · **Requirements:** FR-NEW-013, FR-NEW-043 · **Priority:** Critical
- When `git.token_set` succeeds with token `ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH`
- Then the serialized response contains no substring of length 8 or more from the token

**E2E-NEW-024** — An empty token is rejected
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-015 · **Priority:** Critical
- When `git.token_set` is called with `{"host": "github.ibm.com", "token": ""}`
- Then the call fails with `ERR_INVALID_ARGUMENT`
- And `git.auth_status` reports no token for `github.ibm.com`

**E2E-NEW-025** — A whitespace-only token is rejected
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-015 · **Priority:** High
- When the token is `"   \t\n "`
- Then the call fails with `ERR_INVALID_ARGUMENT` and stores nothing

**E2E-NEW-026** — A token over the bound is rejected
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-015 · **Priority:** High
- When the token is 8193 `a` characters
- Then the call fails with `ERR_INVALID_ARGUMENT` and stores nothing

**E2E-NEW-027** — An undeclared host is rejected for seeding
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-016 · **Priority:** Critical
- When `git.token_set` names `git.unknown.test`
- Then the call fails with `ERR_INVALID_ARGUMENT` naming that host, and stores nothing

**E2E-NEW-028** — An anonymous host is rejected for seeding
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-016 · **Priority:** Critical
- When `git.token_set` names `public.example.org`, declared `anonymous`
- Then the call fails with `ERR_INVALID_ARGUMENT` naming that host
- And the message states that an anonymous host holds no credential

**E2E-NEW-029** — An unauthenticated caller is rejected
- **Category:** Security · **Scenario:** SC-002 · **Requirements:** FR-NEW-013 · **Priority:** Critical
- Given a context whose `person` is empty
- When `git.token_set` is called
- Then the call fails with `ERR_UNAUTHENTICATED`, mirroring `crates/mcp-fs/src/tools/git_auth.rs:143-148`

**E2E-NEW-030** — A failing persistence write fails the call
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-017 · **Priority:** Critical
- Given persistence configured and its backing store made unavailable
- When `git.token_set` is called
- Then the call fails and the response never reports `stored: true`
- And the caller can distinguish this from a validation failure

**E2E-NEW-031** — Seeding twice overwrites, leaving one token
- **Category:** State Transition · **Scenario:** SC-002 · **Requirements:** FR-NEW-013, FR-NEW-009 · **Priority:** High
- When `git.token_set` is called for `github.ibm.com` with `ghp_FIRST…` then `ghp_SECOND…`
- Then exactly one token is stored for `(alice@test.com, github.ibm.com)`
- And a subsequent clone supplies `ghp_SECOND…`

**E2E-NEW-032** — Two hosts of the same provider hold independent tokens
- **Category:** Data Integrity · **Scenario:** SC-002 · **Requirements:** FR-NEW-009 · **Priority:** Critical
- When tokens `ghp_PUBLIC…` and `ghp_ENTERPRISE…` are seeded for `github.com` and `github.ibm.com`
- Then both are retrievable independently
- And a clone from `github.com` supplies `ghp_PUBLIC…` while one from `github.ibm.com` supplies `ghp_ENTERPRISE…`
- *This is the collision that the old `(person, provider)` key made impossible.*

**E2E-NEW-033** — Host is matched case-insensitively on seeding
- **Category:** Edge · **Scenario:** SC-002 · **Requirements:** FR-NEW-009 · **Priority:** High
- When a token is seeded for `GitHub.IBM.com`
- Then `git.auth_status` reports it under `github.ibm.com`
- And a clone of `https://github.ibm.com/o/r.git` uses it

**E2E-NEW-034** — A token at exactly the bound is accepted
- **Category:** Edge · **Scenario:** SC-002 · **Requirements:** FR-NEW-015 · **Priority:** Medium
- When the token is exactly 8192 characters
- Then the call succeeds and the token is stored intact

**E2E-NEW-035** — Memory-only operation is reported explicitly
- **Category:** Edge · **Scenario:** SC-002 · **Requirements:** FR-NEW-014, FR-NEW-017 · **Priority:** High
- Given `MCPFS_TOKEN_KEY` is unset, so persistence is disabled (`FR-628`)
- When `git.token_set` succeeds
- Then the response reports `persistent: false`
- And the response states that the token lives only for the process lifetime

#### Group B — token identity and migration (SC-008)

**E2E-NEW-036** — The persisted key is `(person, host)`
- **Category:** Data Integrity · **Scenario:** SC-008 · **Requirements:** FR-NEW-010 · **Priority:** Critical
- When the schema is created
- Then `oauth_tokens` has primary key columns `person` and `host`, both `TextKey`-bounded
- And `provider` is a non-key column

**E2E-NEW-037** — Legacy rows are dropped on upgrade
- **Category:** State Transition · **Scenario:** SC-008 · **Requirements:** FR-NEW-011 · **Priority:** Critical
- Given an `oauth_tokens` table keyed `(person, provider)` holding rows for `(alice@test.com, github)` and `(bob@test.com, gitlab)`
- When the server starts on the new version
- Then the table is keyed `(person, host)` and contains zero rows
- And `git.auth_status` reports no tokens for either person

**E2E-NEW-038** — Migration is identical on all three backends
- **Category:** Data Integrity · **Scenario:** SC-008 · **Requirements:** FR-NEW-010, FR-NEW-012 · **Priority:** Critical
- When the conformance suite runs on SQLite, PostgreSQL and SQL Server
- Then each produces a table keyed `(person, host)` with `host` indexable
- And the observable behaviour of store, get, revoke and list is identical across all three

**E2E-NEW-039** — A failing migration fails boot, naming the backend
- **Category:** Error · **Scenario:** SC-008 · **Requirements:** FR-NEW-011 · **Priority:** High
- Given a backend that rejects the schema change
- When the server starts
- Then boot fails and the message names the backend

**E2E-NEW-040** — A fresh install performs no drop
- **Category:** Edge · **Scenario:** SC-008 · **Requirements:** FR-NEW-011 · **Priority:** High
- Given no `oauth_tokens` table
- When the server starts
- Then the table is created on the new key and boot succeeds

**E2E-NEW-041** — The migrate verb carries the new key
- **Category:** Data Integrity · **Scenario:** SC-008 · **Requirements:** FR-NEW-012 · **Priority:** High
- Given a source holding tokens keyed `(person, host)`
- When `mcp-fs migrate --from a.yaml --to b.yaml` runs
- Then the destination holds the same rows under the same key, values intact and still decryptable

**E2E-NEW-042** — The migrate verb drops legacy-format source rows
- **Category:** Error · **Scenario:** SC-008 · **Requirements:** FR-NEW-012 · **Priority:** High
- Given a source in the old `(person, provider)` format
- When `migrate` runs
- Then the destination table is on the new key and contains no rows inferred from provider values

**E2E-NEW-043** — Keying is caseless on both parts after the change
- **Category:** Edge · **Scenario:** SC-008 · **Requirements:** FR-NEW-009 · **Priority:** High
- When a token is stored for `("Alice@Test.COM", "GitHub.IBM.com")`
- Then it is retrievable as `("alice@test.com", "github.ibm.com")` and as `("ALICE@TEST.COM", "GITHUB.IBM.COM")`
- And it is not retrievable as `("alice@test.com", "github.com")`

#### Group E — device flow with a host (SC-003)

**E2E-NEW-044** — Device flow stores under the named host
- **Category:** Feature · **Scenario:** SC-003 · **Requirements:** FR-MOD-002 · **Priority:** Critical
- When `git.auth` is called with `{"provider": "github", "host": "github.ibm.com"}` and the fake flow authorises
- Then the token is stored for `(alice@test.com, github.ibm.com)`
- And nothing is stored for `(alice@test.com, github.com)`

**E2E-NEW-045** — An omitted host defaults to the canonical public host
- **Category:** Feature · **Scenario:** SC-003 · **Requirements:** FR-MOD-002 · **Priority:** Critical
- When `git.auth` is called with `{"provider": "github"}` and the flow authorises
- Then the token is stored for `(alice@test.com, github.com)`
- And the same call with `{"provider": "gitlab"}` stores under `gitlab.com`

**E2E-NEW-046** — A host mapped to a different provider is rejected
- **Category:** Error · **Scenario:** SC-003 · **Requirements:** FR-MOD-002 · **Priority:** Critical
- When `git.auth` is called with `{"provider": "gitlab", "host": "github.ibm.com"}`
- Then the call fails with `ERR_INVALID_ARGUMENT` naming both `gitlab` and `github`
- And no device code is requested

**E2E-NEW-047** — A generic host is rejected for the device flow
- **Category:** Error · **Scenario:** SC-003 · **Requirements:** FR-MOD-002 · **Priority:** High
- When `git.auth` names `git.acme.internal`, declared `generic`
- Then the call fails, stating the device flow exists only for `github` and `gitlab`

**E2E-NEW-048** — An anonymous host is rejected for the device flow
- **Category:** Error · **Scenario:** SC-003 · **Requirements:** FR-MOD-002 · **Priority:** High
- When `git.auth` names `public.example.org`
- Then the call fails for the same reason as E2E-NEW-047

**E2E-NEW-049** — An unknown provider is still rejected before any HTTP call
- **Category:** Edge · **Scenario:** SC-003 · **Requirements:** FR-MOD-002 · **Priority:** High
- When `git.auth` is called with `{"provider": "GitHub"}`
- Then the call fails with `ERR_INVALID_ARGUMENT` and no device code is requested
- *Preserves the existing exact-match behaviour at `crates/mcp-fs/src/tools/git_auth.rs:160-162`.*

**E2E-NEW-050** — An undeclared host is rejected for the device flow
- **Category:** Edge · **Scenario:** SC-003 · **Requirements:** FR-MOD-002, FR-NEW-007 · **Priority:** High
- When `git.auth` names `git.unknown.test`
- Then the call fails naming that host, and no device code is requested

#### Group F — clone and origin (SC-004, SC-005)

**E2E-NEW-051** — Clone records origin
- **Category:** Side Effect · **Scenario:** SC-004 · **Requirements:** FR-NEW-020 · **Priority:** Critical
- When `git.remote_clone` succeeds for `https://github.ibm.com/org/repo.git` into `proj-a`
- Then `list_remotes` for `proj-a` returns exactly `[("origin", "https://github.ibm.com/org/repo.git")]`
- *Today no production code calls `add_remote` (`crates/mcp-fs/src/git/db.rs:281`); this test fails before the change.*

**E2E-NEW-052** — Re-cloning replaces the origin row
- **Category:** State Transition · **Scenario:** SC-004 · **Requirements:** FR-NEW-020 · **Priority:** High
- Given `proj-a` cloned from `https://github.ibm.com/org/a.git`
- When it is cloned again from `https://github.ibm.com/org/b.git`
- Then `list_remotes` returns one row, `("origin", "https://github.ibm.com/org/b.git")`

**E2E-NEW-053** — A missing token fails, and does not fall back to anonymous
- **Category:** Error · **Scenario:** SC-004 · **Requirements:** FR-NEW-007 · **Priority:** Critical
- Given `github.ibm.com: github` and no token for `(alice@test.com, github.ibm.com)`
- When a clone is attempted
- Then it fails naming the host and instructing authentication
- And the response does not report `"auth": "anonymous"`, and no clone is attempted

**E2E-NEW-054** — A credential rejected by the remote does not delete the token
- **Category:** Error · **Scenario:** SC-004 · **Requirements:** FR-NEW-019 · **Priority:** High
- Given a stored, unexpired token the remote rejects
- When a clone is attempted
- Then the call fails with the transport error, naming the host
- And the token is still present in the store afterwards

**E2E-NEW-055** — A non-member is refused before any network call
- **Category:** Security · **Scenario:** SC-004 · **Requirements:** FR-NEW-006 · **Priority:** Critical
- Given `bob@test.com` is not a member of `proj-a`
- When bob calls `git.remote_clone` for `proj-a`
- Then the call fails with `ERR_FORBIDDEN`
- And no host resolution, no token lookup and no outbound connection occur

**E2E-NEW-056** — Clone audit records the operation without the credential
- **Category:** Side Effect · **Scenario:** SC-004 · **Requirements:** FR-NEW-043 · **Priority:** Critical
- When a clone succeeds using a stored token
- Then an audit entry exists naming the operation, the mount and the URL
- And the entry contains no substring of the token

**E2E-NEW-057** — A generic host clones with its token
- **Category:** Edge · **Scenario:** SC-004 · **Requirements:** FR-NEW-006 · **Priority:** High
- Given `git.acme.internal: generic` and a token seeded for it
- When a clone of `https://git.acme.internal/o/r.git` runs
- Then the credential `oauth2:<token>` is supplied and the response reports `"auth": "generic"`

**E2E-NEW-058** — The undeclared-host error is distinct from a malformed-URL error
- **Category:** Error · **Scenario:** SC-005 · **Requirements:** FR-NEW-007 · **Priority:** High
- When a clone is attempted with `https://git.unknown.test/o/r.git` and then with `not-a-url`
- Then the two failures carry distinguishable messages
- And the second names malformation, not declaration

**E2E-NEW-059** — A URL with no host is rejected as malformed
- **Category:** Error · **Scenario:** SC-005 · **Requirements:** FR-NEW-007 · **Priority:** High
- When a clone is attempted with `https:///org/repo.git`
- Then the call fails as malformed, and the message does not instruct declaring an empty host

**E2E-NEW-060** — Push, fetch and pull all fail alike on an undeclared host
- **Category:** Error · **Scenario:** SC-005 · **Requirements:** FR-NEW-007 · **Priority:** Critical
- Given a volume whose recorded `origin` is on an undeclared host
- When each of `git.remote_push`, `git.remote_fetch`, `git.remote_pull` is called
- Then all three fail with the same undeclared-host error naming the host, before any network call

**E2E-NEW-061** — A trailing-dot hostname does not match
- **Category:** Edge · **Scenario:** SC-005 · **Requirements:** FR-NEW-006 · **Priority:** Medium
- When a clone is attempted with `https://github.com./o/r.git` while only `github.com` is declared
- Then the behaviour is deterministic and documented: the call fails as an undeclared host

**E2E-NEW-062** — An IDN hostname is handled deterministically
- **Category:** Edge · **Scenario:** SC-005 · **Requirements:** FR-NEW-006 · **Priority:** Medium
- Given `xn--gthub-0na.com: github` declared in punycode
- When a clone of `https://gïthub.com/o/r.git` is attempted
- Then resolution is performed on the punycode form and the outcome matches the declared entry

#### Group E — status and revocation (SC-007)

**E2E-NEW-063** — Status reports one entry per host
- **Category:** Feature · **Scenario:** SC-007 · **Requirements:** FR-MOD-003 · **Priority:** Critical
- Given tokens for `github.com` and `github.ibm.com`
- When `git.auth_status` is called with no arguments
- Then two entries are returned, each with host, provider `github`, validity and expiry

**E2E-NEW-064** — Status filters by provider and by host
- **Category:** Feature · **Scenario:** SC-007 · **Requirements:** FR-MOD-003 · **Priority:** High
- Given tokens for `github.com`, `github.ibm.com` and `gitlab.acme.corp`
- When called with `{"provider": "github"}` then `{"host": "github.ibm.com"}`
- Then the first returns two entries and the second exactly one

**E2E-NEW-065** — Status never includes a token value
- **Category:** Security · **Scenario:** SC-007 · **Requirements:** FR-MOD-003, FR-NEW-043 · **Priority:** Critical
- When `git.auth_status` returns entries for seeded tokens
- Then the serialized response contains no substring of length 8 or more from any stored token

**E2E-NEW-066** — Revoke removes exactly one host
- **Category:** State Transition · **Scenario:** SC-007 · **Requirements:** FR-MOD-004 · **Priority:** Critical
- Given tokens for `github.com` and `github.ibm.com`
- When `git.auth_revoke` is called with `{"provider": "github", "host": "github.ibm.com"}`
- Then `github.ibm.com` is gone from memory and persistence
- And the `github.com` token is untouched and still usable for a clone

**E2E-NEW-067** — Revoke with an omitted host targets only the canonical host
- **Category:** Error · **Scenario:** SC-007 · **Requirements:** FR-MOD-004 · **Priority:** Critical
- Given tokens for `github.com` and `github.ibm.com`
- When `git.auth_revoke` is called with `{"provider": "github"}`
- Then only `github.com` is revoked
- And `github.ibm.com` remains, proving revocation never crosses hosts

**E2E-NEW-068** — Revoking an absent host is idempotent
- **Category:** Edge · **Scenario:** SC-007 · **Requirements:** FR-MOD-004 · **Priority:** High
- When `git.auth_revoke` is called twice for `github.ibm.com`
- Then both calls succeed and the second reports nothing revoked

**E2E-NEW-069** — Status with no tokens returns an empty list
- **Category:** Edge · **Scenario:** SC-007 · **Requirements:** FR-MOD-003 · **Priority:** High
- When `git.auth_status` is called by a person holding no tokens
- Then it succeeds and returns an empty collection, not an error

**E2E-NEW-070** — Status distinguishes expired from absent
- **Category:** Edge · **Scenario:** SC-007 · **Requirements:** FR-MOD-003, FR-NEW-019 · **Priority:** Critical
- Given an expired token for `github.ibm.com` and nothing for `gitlab.acme.corp`
- When `git.auth_status` is called
- Then `github.ibm.com` appears with validity `expired`
- And `gitlab.acme.corp` does not appear at all

#### Group G — push (SC-009, SC-010)

**E2E-NEW-071** — Push updates an existing remote branch
- **Category:** Core Journey · **Scenario:** SC-009 · **Requirements:** FR-NEW-022 · **Priority:** Critical
- Given `proj-a` cloned from `https://github.ibm.com/org/repo.git`, one new local commit on `main`
- When `git.remote_push` is called with `{"mount_id": "proj-a", "branch": "main"}`
- Then the response reports `{"branch": "main", "created": false, "remote_sha": <local tip>, "auth": "github"}`
- And the remote's `refs/heads/main` equals the local tip

**E2E-NEW-072** — Push creates a branch absent on the remote
- **Category:** State Transition · **Scenario:** SC-009 · **Requirements:** FR-NEW-023 · **Priority:** Critical
- Given a local branch `feature/x` that the remote does not have
- When it is pushed
- Then the response reports `created: true`
- And the remote holds `refs/heads/feature/x` at the local tip

**E2E-NEW-073** — Push leaves the volume unchanged
- **Category:** Side Effect · **Scenario:** SC-009 · **Requirements:** FR-NEW-022 · **Priority:** High
- When a push succeeds
- Then every file in the volume is byte-for-byte identical to before
- And `refs/heads/main` is unchanged locally

**E2E-NEW-074** — Push is idempotent when already up to date
- **Category:** Feature · **Scenario:** SC-009 · **Requirements:** FR-NEW-025 · **Priority:** Critical
- When `git.remote_push` is called twice with no commit in between
- Then both succeed and the second reports `up_to_date: true`

**E2E-NEW-075** — Push without an origin is rejected
- **Category:** Error · **Scenario:** SC-009 · **Requirements:** FR-NEW-021 · **Priority:** Critical
- Given a volume created by `git.init` with a commit and no recorded remote
- When `git.remote_push` is called
- Then it fails stating the volume has no `origin` remote
- And no network call occurs

**E2E-NEW-076** — A non-fast-forward push is refused with a distinct error
- **Category:** Error · **Scenario:** SC-010 · **Requirements:** FR-NEW-024 · **Priority:** Critical
- Given the remote `main` holds a commit the volume lacks
- When `git.remote_push` is called for `main`
- Then it fails with an error stating the push was refused as non-fast-forward and that force is not supported
- And the remote's `refs/heads/main` is unchanged

**E2E-NEW-077** — The non-fast-forward error is distinguishable from an auth error
- **Category:** Error · **Scenario:** SC-010 · **Requirements:** FR-NEW-024 · **Priority:** High
- When a non-fast-forward push and a credential-rejected push are each attempted
- Then the two failures carry different, machine-distinguishable error identities

**E2E-NEW-078** — A branch absent locally is rejected
- **Category:** Error · **Scenario:** SC-009 · **Requirements:** FR-NEW-022 · **Priority:** High
- When `git.remote_push` names `no-such-branch`
- Then it fails naming that branch, before any network call

**E2E-NEW-079** — A protected-branch rejection surfaces the remote's reason
- **Category:** Error · **Scenario:** SC-009 · **Requirements:** FR-NEW-022 · **Priority:** High
- Given the remote rejects `main` with a protected-branch message
- When the push runs
- Then the failure includes the remote's own reason and names the branch

**E2E-NEW-080** — Push on a non-git volume is rejected
- **Category:** Error · **Scenario:** SC-009 · **Requirements:** FR-NEW-021 · **Priority:** High
- Given a volume never initialised as a repository
- When `git.remote_push` is called
- Then it fails cleanly, with no panic and no partial state

**E2E-NEW-081** — A non-member cannot push
- **Category:** Security · **Scenario:** SC-009 · **Requirements:** FR-NEW-022 · **Priority:** Critical
- When `bob@test.com`, not a member of `proj-a`, pushes
- Then it fails with `ERR_FORBIDDEN` before host resolution or any token lookup

**E2E-NEW-082** — Push uses the pusher's own token, not the cloner's
- **Category:** Security · **Scenario:** SC-009 · **Requirements:** FR-NEW-009 · **Priority:** Critical
- Given `proj-a` cloned by `alice@test.com`, and `bob@test.com` a member holding his own token for the same host
- When bob pushes
- Then the credential supplied is bob's token, never alice's

**E2E-NEW-083** — Push to an anonymous host attempts with no credential
- **Category:** Edge · **Scenario:** SC-009 · **Requirements:** FR-NEW-008 · **Priority:** Medium
- Given an `origin` on `public.example.org`, declared `anonymous`
- When a push runs
- Then no credential is supplied and the remote's rejection, if any, is surfaced verbatim

**E2E-NEW-084** — Concurrent pushes serialize
- **Category:** Edge · **Scenario:** SC-009 · **Requirements:** FR-NEW-022 · **Priority:** High
- When two pushes to `main` on `proj-a` are issued concurrently
- Then they serialize on the per-repository write lock (`FR-609`)
- And the second either reports up to date or fails as non-fast-forward, never corrupting state

**E2E-NEW-085** — A push racing a remote update fails cleanly
- **Category:** Edge · **Scenario:** SC-010 · **Requirements:** FR-NEW-024 · **Priority:** Medium
- Given the remote advances between the pipeline reading it and the push
- When the push runs
- Then the remote's rejection is authoritative and the same non-fast-forward error is produced

#### Group H — fetch (SC-011)

**E2E-NEW-086** — Fetch updates remote-tracking refs
- **Category:** Feature · **Scenario:** SC-011 · **Requirements:** FR-NEW-026 · **Priority:** Critical
- Given the remote `main` two commits ahead
- When `git.remote_fetch` is called
- Then `refs/remotes/origin/main` equals the remote tip
- And the response lists that ref and a non-zero object count

**E2E-NEW-087** — Fetch downloads the objects
- **Category:** Side Effect · **Scenario:** SC-011 · **Requirements:** FR-NEW-026 · **Priority:** Critical
- When a fetch brings two commits
- Then both commit objects and their trees and blobs are present in the volume's object store
- And `git.show` can read the fetched commit

**E2E-NEW-088** — Fetch leaves local refs and files untouched
- **Category:** Side Effect · **Scenario:** SC-011 · **Requirements:** FR-NEW-027 · **Priority:** Critical
- Given a snapshot of every file in the volume and of `refs/heads/main`
- When a fetch runs against a remote that is ahead
- Then every file is byte-for-byte identical afterwards
- And `refs/heads/main` is unchanged

**E2E-NEW-089** — Fetch without an origin is rejected
- **Category:** Error · **Scenario:** SC-011 · **Requirements:** FR-NEW-021 · **Priority:** Critical
- Given a volume from `git.init`
- When `git.remote_fetch` is called
- Then it fails stating the volume has no `origin`

**E2E-NEW-090** — Fetch with a missing token is rejected before the network
- **Category:** Error · **Scenario:** SC-011 · **Requirements:** FR-NEW-007 · **Priority:** Critical
- Given no token for the origin's host, which is declared `github`
- When a fetch runs
- Then it fails naming the host, with no outbound connection

**E2E-NEW-091** — An unreachable remote fails naming the host
- **Category:** Error · **Scenario:** SC-011 · **Requirements:** FR-NEW-026 · **Priority:** High
- Given the remote refuses connections
- When a fetch runs
- Then the failure names the host and is distinguishable from a credential failure

**E2E-NEW-092** — Fetch when already current is idempotent
- **Category:** Edge · **Scenario:** SC-011 · **Requirements:** FR-NEW-026 · **Priority:** High
- When a fetch runs twice with no remote change
- Then both succeed and the second reports zero refs updated

**E2E-NEW-093** — Fetch of a deleted remote branch is handled
- **Category:** Edge · **Scenario:** SC-011 · **Requirements:** FR-NEW-026 · **Priority:** Medium
- Given `origin/feature-x` existed and the remote deleted it
- When a fetch runs
- Then the outcome is deterministic and reported, and no local ref or file changes

**E2E-NEW-094** — Fetch of a new remote branch creates only a remote-tracking ref
- **Category:** Edge · **Scenario:** SC-011 · **Requirements:** FR-NEW-027 · **Priority:** High
- Given the remote gained `feature-y`
- When a fetch runs
- Then `refs/remotes/origin/feature-y` exists
- And no `refs/heads/feature-y` is created and no file appears in the volume

#### Group D — expiry (SC-014)

**E2E-NEW-095** — Clone with an expired token fails before the network
- **Category:** Error · **Scenario:** SC-014 · **Requirements:** FR-NEW-018 · **Priority:** Critical
- Given a token for `(alice@test.com, github.ibm.com)` expiring one hour ago
- When `git.remote_clone` runs
- Then the call fails naming `github.ibm.com` and instructing re-authentication
- And zero outbound connections were attempted
- *Today the clone path uses `get_token` (`crates/mcp-fs/src/tools/git.rs:704`), which ignores expiry (`crates/mcp-fs/src/git/oauth/store.rs:148-150`); this test fails before the change.*

**E2E-NEW-096** — Push, fetch and pull all enforce expiry
- **Category:** Error · **Scenario:** SC-014 · **Requirements:** FR-NEW-018 · **Priority:** Critical
- Given an expired token for the origin's host
- When each of push, fetch and pull runs
- Then all three fail with the same expiry error and make no network call

**E2E-NEW-097** — An expired token survives the failure
- **Category:** Security · **Scenario:** SC-014 · **Requirements:** FR-NEW-019 · **Priority:** Critical
- When an operation fails per E2E-NEW-095
- Then the token is still stored, and `git.auth_status` reports `github.ibm.com` as `expired`

**E2E-NEW-098** — The expiry error differs from the missing-token error
- **Category:** Error · **Scenario:** SC-014 · **Requirements:** FR-NEW-018 · **Priority:** High
- When one operation runs with an expired token and another with none
- Then the two failures are machine-distinguishable, so a caller can decide whether to re-seed or to authenticate

**E2E-NEW-099** — A token expiring exactly now is treated as expired
- **Category:** Edge · **Scenario:** SC-014 · **Requirements:** FR-NEW-018 · **Priority:** Medium
- Given `expires_at` equal to the current instant
- When an operation runs
- Then the boundary is handled per `OAuthSession::is_valid_at` (`crates/mcp-fs/src/git/oauth/store.rs:34`) and the outcome is asserted explicitly rather than left to chance

#### Group J — the web token screen (SC-006)

**E2E-NEW-100** — The screen lists held hosts
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-038 · **Priority:** Critical
- Given `alice@test.com` holds a valid token for `github.com` and an expired one for `github.ibm.com`
- When she requests the token screen route with a valid RS256 JWT
- Then the response lists two rows: `github.com / github / valid` and `github.ibm.com / github / expired`

**E2E-NEW-101** — The screen seeds a token
- **Category:** Core Journey · **Scenario:** SC-006 · **Requirements:** FR-NEW-039 · **Priority:** Critical
- When alice submits a token for `gitlab.acme.corp` through the screen
- Then `git.auth_status` subsequently reports that host as `valid`
- And a clone from that host uses the submitted token

**E2E-NEW-102** — The screen revokes a token
- **Category:** Core Journey · **Scenario:** SC-006 · **Requirements:** FR-NEW-039 · **Priority:** Critical
- Given alice holds tokens for `github.com` and `github.ibm.com`
- When she revokes `github.ibm.com` through the screen
- Then only that token is gone and `github.com` remains

**E2E-NEW-103** — The screen offers only declared hosts
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-038 · **Priority:** High
- When the screen renders its host selector under the reference map
- Then it offers `github.com`, `github.ibm.com`, `gitlab.acme.corp` and `git.acme.internal`
- And it does not offer `public.example.org`, which is `anonymous`

**E2E-NEW-104** — No token value reaches the browser
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-038, FR-NEW-043 · **Priority:** Critical
- Given alice holds `ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH` for `github.com`
- When the screen is rendered and every asset it references is fetched
- Then no response body contains any substring of length 8 or more from that token

**E2E-NEW-105** — An unauthenticated request is rejected
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-037 · **Priority:** Critical
- When the route is requested with no `Authorization` header
- Then the identity layer rejects it and no token metadata is returned

**E2E-NEW-106** — An invalid or expired JWT is rejected
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-037 · **Priority:** Critical
- When the route is requested with a JWT signed by the wrong key, then with one expired beyond the 30-second skew
- Then both are rejected, consistent with `crates/mcp-fs/src/identity.rs`

**E2E-NEW-107** — One person cannot read another's tokens
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-040 · **Priority:** Critical
- Given alice holds a token for `github.ibm.com`
- When bob requests the screen, including any parameter naming alice
- Then the response contains nothing about alice's tokens and the request is refused

**E2E-NEW-108** — A platform admin cannot read another person's tokens
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-040 · **Priority:** Critical
- Given bob holds the platform admin claim and alice holds a token
- When bob attempts to view or revoke alice's token through the screen
- Then the attempt fails with `ERR_FORBIDDEN`
- *Platform admin manages projects and membership and gains no implicit access.*

**E2E-NEW-109** — Seeding an undeclared host through the screen is rejected
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-038 · **Priority:** Critical
- When a request is submitted directly for `git.unknown.test`, bypassing the rendered selector
- Then the server rejects it naming the host, proving validation is server-side

**E2E-NEW-110** — An empty token submitted through the screen is rejected
- **Category:** Error · **Scenario:** SC-006 · **Requirements:** FR-NEW-039 · **Priority:** High
- When an empty value is submitted for `github.com`
- Then it is rejected under the same rule as `git.token_set`, and nothing is stored

**E2E-NEW-111** — The screen cannot modify the host map
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-005 · **Priority:** Critical
- When every request the screen can issue is enumerated
- Then none changes a host-to-provider entry

**E2E-NEW-112** — The screen and the tools agree
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-039 · **Priority:** High
- When a token is seeded through the screen and then through `git.token_set` for another host
- Then `git.auth_status` reports both identically, proving one implementation behind two adapters

**E2E-NEW-113** — Injection in a submitted value is neutralised
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-039 · **Priority:** High
- When a token containing `<script>alert(1)</script>` and one containing `'; DROP TABLE oauth_tokens; --` are submitted
- Then neither is executed nor reflected unescaped, the table still exists, and the values round-trip intact if stored

#### Group I — pull and merge (SC-012, SC-013, SC-015)

**E2E-NEW-114** — A fast-forward pull advances the branch
- **Category:** Core Journey · **Scenario:** SC-012 · **Requirements:** FR-NEW-028 · **Priority:** Critical
- Given the local `main` two commits behind and the volume clean
- When `git.remote_pull` is called with `{"mount_id": "proj-a", "branch": "main"}`
- Then the response reports the old sha, the new sha equal to the remote tip, and `merged: false`
- And `refs/heads/main` equals the remote tip

**E2E-NEW-115** — A fast-forward pull updates the volume's files
- **Category:** Side Effect · **Scenario:** SC-012 · **Requirements:** FR-NEW-028 · **Priority:** Critical
- Given the incoming commits add `/docs/new.md` and modify `/README.md`
- When the pull runs
- Then `/docs/new.md` exists with the incoming content and `/README.md` matches the new tip
- And the reported `files_changed` equals 2

**E2E-NEW-116** — A fast-forward pull creates no merge commit
- **Category:** State Transition · **Scenario:** SC-012 · **Requirements:** FR-NEW-028 · **Priority:** High
- When a fast-forward pull completes
- Then the new tip is the fetched remote commit itself, with its original parent count and sha
- And no commit authored by the server exists

**E2E-NEW-117** — A diverged pull with no strategy is refused
- **Category:** Error · **Scenario:** SC-013 · **Requirements:** FR-NEW-030 · **Priority:** Critical
- Given local and remote `main` have each gained a distinct commit, and `on_conflict` is omitted
- When the pull runs
- Then it fails with an error stating the pull is not a fast-forward and naming `on_conflict`
- And `refs/heads/main` and every volume file are unchanged

**E2E-NEW-118** — A diverged pull with `theirs` merges
- **Category:** Core Journey · **Scenario:** SC-015 · **Requirements:** FR-NEW-031, FR-NEW-034 · **Priority:** Critical
- Given both sides modified `/src/lib.rs`, local to `fn local()` and remote to `fn remote()`
- When the pull runs with `{"on_conflict": "theirs"}`
- Then a merge commit is created with two parents, the first the previous local tip and the second the fetched remote tip
- And `/src/lib.rs` contains `fn remote()`
- And the response reports `{"merged": true, "strategy": "theirs", "conflicts_resolved": 1}`

**E2E-NEW-119** — Pull with an expired token fails before fetching
- **Category:** Edge · **Scenario:** SC-014 · **Requirements:** FR-NEW-018 · **Priority:** Critical
- Given an expired token for the origin's host
- When a pull runs
- Then it fails with the expiry error, no fetch occurs, and no remote-tracking ref changes

**E2E-NEW-120** — A dirty volume refuses the pull
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-029 · **Priority:** Critical
- Given `fs.write` has modified `/README.md` since the last commit
- When a pull that would fast-forward is attempted
- Then it is refused, instructing the caller to commit or discard first
- And the modified `/README.md` retains the uncommitted content
- And no fetch occurred

**E2E-NEW-121** — A volume with an added uncommitted file is dirty
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-029 · **Priority:** High
- Given `/scratch.txt` was written and never committed
- When a pull is attempted
- Then it is refused as dirty, and `/scratch.txt` survives untouched

**E2E-NEW-122** — A volume with a deleted uncommitted file is dirty
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-029 · **Priority:** High
- Given a tracked file was deleted and not committed
- When a pull is attempted
- Then it is refused as dirty

**E2E-NEW-123** — Committing then pulling succeeds via merge
- **Category:** State Transition · **Scenario:** SC-012 · **Requirements:** FR-NEW-029, FR-NEW-031 · **Priority:** Critical
- Given a dirty volume and a remote that has advanced
- When the caller commits, then pulls with `{"on_conflict": "ours"}`
- Then the pull succeeds with a merge commit
- *This proves the "commit or discard" guidance is not a dead end: the committed divergence is resolvable.*

**E2E-NEW-124** — Pull without an origin is rejected
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-021 · **Priority:** Critical
- Given a volume from `git.init`
- When a pull is attempted
- Then it fails stating the volume has no `origin`

**E2E-NEW-125** — A refused diverged pull keeps the fetched objects
- **Category:** Side Effect · **Scenario:** SC-013 · **Requirements:** FR-NEW-030 · **Priority:** Critical
- When a pull is refused per E2E-NEW-117
- Then `refs/remotes/origin/main` equals the remote tip
- And the fetched commit objects are present and readable by `git.show`
- *The fetch genuinely happened; only the apply step was refused.*

**E2E-NEW-126** — The diverged-pull error names the remedy
- **Category:** Error · **Scenario:** SC-013 · **Requirements:** FR-NEW-030 · **Priority:** High
- When a pull is refused as diverged
- Then the message names the `on_conflict` parameter and its accepted values

**E2E-NEW-127** — The diverged error differs from the dirty error
- **Category:** Error · **Scenario:** SC-013 · **Requirements:** FR-NEW-029, FR-NEW-030 · **Priority:** High
- When one pull is refused as dirty and another as diverged
- Then the two failures are machine-distinguishable

**E2E-NEW-128** — A diverged pull with `ours` keeps local content
- **Category:** Feature · **Scenario:** SC-015 · **Requirements:** FR-NEW-031 · **Priority:** Critical
- Given both sides modified `/src/lib.rs`
- When the pull runs with `{"on_conflict": "ours"}`
- Then `/src/lib.rs` contains the local content
- And a merge commit still exists with both parents

**E2E-NEW-129** — Non-conflicting changes from both sides are both kept
- **Category:** Data Integrity · **Scenario:** SC-015 · **Requirements:** FR-NEW-031 · **Priority:** Critical
- Given local added `/a.txt` and remote added `/b.txt`, with no common file touched
- When the pull runs with `{"on_conflict": "ours"}`
- Then both `/a.txt` and `/b.txt` exist afterwards
- And `conflicts_resolved` is 0
- *The strategy applies only to conflicting files; it must not discard the other side's non-conflicting work.*

**E2E-NEW-130** — A merged branch then pushes as a fast-forward
- **Category:** Edge · **Scenario:** SC-015 · **Requirements:** FR-NEW-031, FR-NEW-022 · **Priority:** Critical
- Given a merge completed per E2E-NEW-118
- When `git.remote_push` is called for `main`
- Then it succeeds as a fast-forward
- *This closes the loop: divergence is recoverable end to end.*

**E2E-NEW-131** — An up-to-date pull is idempotent
- **Category:** Edge · **Scenario:** SC-012 · **Requirements:** FR-NEW-028 · **Priority:** High
- When a pull runs twice with no remote change
- Then both succeed, the second reporting no change and `files_changed` of 0

**E2E-NEW-132** — Pull is atomic when a file write fails
- **Category:** Edge · **Scenario:** SC-012 · **Requirements:** FR-NEW-035 · **Priority:** Critical
- Given a fast-forward whose tree contains one path the volume cannot write
- When the pull runs
- Then it fails naming that path
- And `refs/heads/main` is unchanged and every other file retains its pre-pull content
- *Deliberately unlike clone, which reports `skipped` and continues (`crates/mcp-fs/src/tools/git.rs:785-791`).*

**E2E-NEW-133** — Pull charges the write quota before writing
- **Category:** Performance · **Scenario:** SC-012 · **Requirements:** FR-NEW-036 · **Priority:** Critical
- Given `safety.write_quota_bytes` lower than the summed size of the blobs the pull would actually write
- When a pull is attempted
- Then it fails with a quota error, no file is written, and the ref is not advanced

**E2E-NEW-134** — A degenerate diverged pull is a fast-forward
- **Category:** Edge · **Scenario:** SC-013 · **Requirements:** FR-NEW-028 · **Priority:** Medium
- Given the local branch turns out to be a strict ancestor after fetching
- When the pull runs with `on_conflict` supplied
- Then it applies as a fast-forward and creates no merge commit

**E2E-NEW-135** — An invalid conflict strategy is rejected
- **Category:** Error · **Scenario:** SC-015 · **Requirements:** FR-NEW-033 · **Priority:** Critical
- When `on_conflict` is `union`, `OURS`, `Theirs`, `""` or `normal`
- Then each is rejected with `ERR_INVALID_ARGUMENT` naming `ours` and `theirs`
- And nothing is applied

**E2E-NEW-136** — No conflict markers enter the volume
- **Category:** Data Integrity · **Scenario:** SC-015 · **Requirements:** FR-NEW-032 · **Priority:** Critical
- Given a merge resolving three conflicting files
- When the pull completes with either strategy
- Then no file in the volume contains `<<<<<<<`, `=======` or `>>>>>>>` introduced by the merge

**E2E-NEW-137** — Merge is atomic when a write fails
- **Category:** Error · **Scenario:** SC-015 · **Requirements:** FR-NEW-035 · **Priority:** Critical
- Given a merge whose tree contains one unwritable path
- When the pull runs
- Then no merge commit is created, the ref is not advanced, and every file retains its pre-merge content

**E2E-NEW-138** — A dirty volume refuses the merge too
- **Category:** Error · **Scenario:** SC-015 · **Requirements:** FR-NEW-029 · **Priority:** Critical
- Given a dirty volume and a diverged remote, with `on_conflict` supplied
- When the pull runs
- Then it is refused as dirty before any merge is attempted

**E2E-NEW-139** — A conflict-free merge still creates a merge commit
- **Category:** Edge · **Scenario:** SC-015 · **Requirements:** FR-NEW-031 · **Priority:** High
- Given diverged branches with no overlapping file
- When the pull runs with `{"on_conflict": "ours"}`
- Then a merge commit with two parents is created and `conflicts_resolved` is 0

**E2E-NEW-140** — The merge commit records author and strategy
- **Category:** Edge · **Scenario:** SC-015 · **Requirements:** FR-NEW-034 · **Priority:** Critical
- When `alice@test.com` merges `origin/main` into `main` with `theirs`
- Then the commit's author and committer are `alice@test.com`
- And its message is exactly `Merge origin/main into main (conflicts resolved: theirs)`
- And `git.log` shows that message

**E2E-NEW-141** — A merge exceeding the quota is refused
- **Category:** Edge · **Scenario:** SC-015 · **Requirements:** FR-NEW-036 · **Priority:** High
- Given the merged tree's changed blobs exceed the write quota
- When the pull runs
- Then it fails before writing, creates no merge commit, and leaves the ref unchanged

#### Group K — cross-cutting security

**E2E-NEW-142** — An HTTPS URL is accepted
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-041 · **Priority:** Critical
- When a clone of `https://github.ibm.com/o/r.git` runs against a declared host
- Then the scheme check passes and the operation proceeds

**E2E-NEW-143** — An `ssh://` URL is rejected
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-041 · **Priority:** Critical
- When a clone of `ssh://git@github.com/o/r.git` is attempted
- Then it fails with `ERR_INVALID_ARGUMENT` naming the scheme, with no network call

**E2E-NEW-144** — A `git://` URL is rejected
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-041 · **Priority:** Critical
- When `git://github.com/o/r.git` is attempted
- Then it fails naming the scheme

**E2E-NEW-145** — A `file://` URL is rejected
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-041 · **Priority:** Critical
- When `file:///etc/passwd` and `file:///tmp/repo` are attempted
- Then both fail, and no path on the server's filesystem is read
- *A `file://` remote would let a caller read the server's own disk.*

**E2E-NEW-146** — A plain `http://` URL is rejected
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-041 · **Priority:** Critical
- When `http://github.com/o/r.git` is attempted
- Then it fails, because a token must never travel unencrypted

**E2E-NEW-147** — The `scp`-style shorthand is rejected
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-041 · **Priority:** High
- When `git@github.com:org/repo.git` is attempted
- Then it fails as not HTTPS, and is not silently reinterpreted as a hostname

**E2E-NEW-148** — A URL carrying userinfo is rejected
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-042 · **Priority:** Critical
- When `https://alice:ghp_secret@github.com/o/r.git` is attempted
- Then it fails with `ERR_INVALID_ARGUMENT`
- And the error message contains neither `ghp_secret` nor the full URL

**E2E-NEW-149** — A rejected userinfo URL is never stored as origin
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-042 · **Priority:** Critical
- When the clone of E2E-NEW-148 is attempted
- Then `list_remotes` for the volume returns no row containing `ghp_secret`

**E2E-NEW-150** — A rejected userinfo URL never reaches the audit log
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-042, FR-NEW-043 · **Priority:** Critical
- When the clone of E2E-NEW-148 is attempted
- Then no audit entry contains `ghp_secret`
- *The clone audit entry records the URL today (`crates/mcp-fs/src/tools/git.rs:794-800`), which is why this rejection must happen first.*

**E2E-NEW-151** — Tracing output never contains a token
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-043 · **Priority:** Critical
- Given a tracing subscriber capturing every event at TRACE level
- When clone, push, fetch and pull each run successfully with a stored token
- Then no captured event contains any substring of length 8 or more from that token

**E2E-NEW-152** — Error messages never contain a token
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-043 · **Priority:** Critical
- When each failure mode in this specification is triggered with a token present
- Then no error message contains any substring of the token, including messages originating in libgit2

**E2E-NEW-153** — Audit entries never contain a token
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-043 · **Priority:** Critical
- When every remote operation runs
- Then each records an audit entry naming the operation, host, branch and outcome
- And none contains a token value

**E2E-NEW-154** — `Debug` output stays redacted under the new key
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-043 · **Priority:** High
- When an `OAuthSession` is formatted with `{:?}`
- Then the token is redacted, preserving the behaviour at `crates/mcp-fs/src/git/oauth/store.rs:41`

**E2E-NEW-155** — A remote operation times out
- **Category:** Performance · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-044 · **Priority:** Critical
- Given `git.remote_timeout_secs: 5` and a remote that accepts the connection and never responds
- When a clone runs
- Then it fails within approximately five seconds with a distinct timeout error naming the host

**E2E-NEW-156** — A timeout releases the repository lock
- **Category:** Performance · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-044 · **Priority:** Critical
- Given the timeout of E2E-NEW-155 has fired
- When another git operation on the same volume is issued
- Then it proceeds without waiting, proving the per-repository write lock was released

**E2E-NEW-157** — The timeout default is 120 and is configurable
- **Category:** Performance · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-044 · **Priority:** Medium
- When the config omits `git.remote_timeout_secs`
- Then the effective value is 120
- And setting it to 5 changes the effective value to 5

#### Group L — contract

**E2E-NEW-158** — All new tools are registered
- **Category:** Integration · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-045 · **Priority:** Critical
- When the registry is enumerated with git enabled
- Then `git.token_set`, `git.remote_push`, `git.remote_fetch` and `git.remote_pull` are present
- And none is present when git is disabled, consistent with `FR-611`

**E2E-NEW-159** — New tool schemas match the frozen contract
- **Category:** Integration · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-045 · **Priority:** Critical
- When each new tool's `inputSchema` is compared to `tool-contract-golden.json`
- Then each matches exactly, including key order

**E2E-NEW-160** — Modified tool schemas match the frozen contract
- **Category:** Integration · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-045 · **Priority:** Critical
- When `git.auth`, `git.auth_status` and `git.auth_revoke` are compared to the golden file
- Then each matches, each exposes `host` as optional, and no previously required parameter became optional or vice versa

**E2E-NEW-161** — Existing callers keep working without `host`
- **Category:** Integration · **Scenario:** Cross-cutting · **Requirements:** FR-MOD-002, FR-MOD-003, FR-MOD-004 · **Priority:** Critical
- When `git.auth`, `git.auth_status` and `git.auth_revoke` are each called with exactly the arguments valid before this change
- Then all three succeed, defaulting to the canonical public host

#### Added at review

**E2E-NEW-162** — Two people hold independent tokens for one host
- **Category:** Security · **Scenario:** SC-006, SC-008 · **Requirements:** FR-NEW-009, FR-NEW-040 · **Priority:** Critical
- Given alice and bob each seed a different token for `github.ibm.com`
- Then both are stored and retrievable independently
- And a clone by alice supplies alice's token and one by bob supplies bob's
- And neither person's `git.auth_status` or token screen reveals the other's

**E2E-NEW-163** — Revocation by one person does not affect another
- **Category:** Data Integrity · **Scenario:** SC-008 · **Requirements:** FR-NEW-009 · **Priority:** Critical
- Given the state of E2E-NEW-162
- When alice revokes `github.ibm.com`
- Then bob's token for the same host remains and still authenticates a clone


#### Group M — amendments from the implementability audit

**E2E-NEW-164** — The screen route serves HTML to an authenticated person
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-046 · **Priority:** Critical
- When `GET /app/tokens` is requested with a verified identity
- Then the response is `200` with `Content-Type: text/html`

**E2E-NEW-165** — Seeding through the route redirects
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-046 · **Priority:** Critical
- When `POST /app/tokens` is submitted with `host=github.ibm.com&token=ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH`
- Then the response is `303` with `Location: /app/tokens`
- And `git.auth_status` subsequently lists `github.ibm.com`

**E2E-NEW-166** — Revocation through the route redirects
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-046 · **Priority:** Critical
- When `POST /app/tokens/revoke` is submitted with `host=github.ibm.com`
- Then the response is `303` and the token is gone

**E2E-NEW-167** — A rejected host returns 400 with the error body
- **Category:** Error · **Scenario:** SC-006 · **Requirements:** FR-NEW-046 · **Priority:** Critical
- When `POST /app/tokens` is submitted with `host=git.unknown.test&token=ghp_x`
- Then the response is `400` with body `{"error":"ERR_INVALID_ARGUMENT","detail":"..."}` naming the host

**E2E-NEW-168** — The screen routes are absent when git is disabled
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-046 · **Priority:** High
- Given `git.enabled` is false
- When `GET /app/tokens` is requested
- Then the route is not registered and the response is `404`

**E2E-NEW-169** — Identity resolves from the forwarded header
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** Critical
- When `GET /app/tokens` carries `X-Forwarded-Authorization: Bearer <valid jwt for alice@test.com>`
- Then the screen renders scoped to `alice@test.com`

**E2E-NEW-170** — Identity resolves from the `Authorization` header
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** Critical
- When the same request carries `Authorization: Bearer <valid jwt>` instead
- Then the screen renders identically

**E2E-NEW-171** — Identity resolves from the cookie
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** Critical
- When the request carries only `Cookie: mcpfs_token=<valid jwt>` and no bearer header
- Then the screen renders scoped to that person
- *This is the only path a browser navigation can take.*

**E2E-NEW-172** — A request with no credential is rejected
- **Category:** Error · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** Critical
- When `GET /app/tokens` carries neither header nor cookie
- Then the response is `401` with body `{"error":"ERR_UNAUTHENTICATED","detail":"..."}`

**E2E-NEW-173** — A cookie holding an invalid JWT is rejected
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** Critical
- When the cookie holds a JWT signed by the wrong key, then one expired beyond the 30-second skew
- Then both yield `401`, proving the cookie is verified exactly as a header bearer token is

**E2E-NEW-174** — No route outside the screen accepts the cookie
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** Critical
- When an `/api/fs` request and an MCP request each carry only `Cookie: mcpfs_token=<valid jwt>`
- Then both are rejected as unauthenticated

**E2E-NEW-175** — The server never sets the cookie
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** High
- When every response from every route is inspected
- Then none carries a `Set-Cookie` header for `mcpfs_token`

**E2E-NEW-176** — A cookie-authenticated POST without the anti-forgery token is refused
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-048 · **Priority:** Critical
- When `POST /app/tokens` carries the cookie and no anti-forgery field
- Then the response is `403` and no credential is stored

**E2E-NEW-177** — A POST carrying the issued anti-forgery token succeeds
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-048 · **Priority:** Critical
- Given `GET /app/tokens` issued an anti-forgery token bound to `alice@test.com`
- When `POST /app/tokens` carries the cookie and that token
- Then the seeding succeeds

**E2E-NEW-178** — Another person's anti-forgery token is refused
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-048 · **Priority:** Critical
- When alice submits with an anti-forgery token issued to `bob@test.com`
- Then the response is `403` and nothing is stored

**E2E-NEW-179** — An anti-forgery token is single use
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-048 · **Priority:** High
- When the same anti-forgery token is replayed on a second `POST`
- Then the second is refused with `403`

**E2E-NEW-180** — A header-authenticated POST is exempt
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-048 · **Priority:** High
- When `POST /app/tokens` is authenticated by `Authorization` rather than the cookie, with no anti-forgery field
- Then it succeeds, because no ambient credential was used

**E2E-NEW-181** — The non-fast-forward push message prefix is exact
- **Category:** Error · **Scenario:** SC-010 · **Requirements:** FR-NEW-049 · **Priority:** Critical
- When a push is refused as non-fast-forward
- Then the error is `ERR_INVALID_ARGUMENT` and the message begins `push refused: not a fast-forward`

**E2E-NEW-182** — The dirty-volume and diverged-pull prefixes are exact and different
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-049 · **Priority:** Critical
- When a pull is refused for a dirty volume, then for divergence with no strategy
- Then the first begins `pull refused: volume has uncommitted changes` and the second `pull refused: not a fast-forward`

**E2E-NEW-183** — The timeout code and prefix are exact
- **Category:** Error · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-049 · **Priority:** Critical
- When a remote operation exceeds the timeout
- Then the error is `ERR_INTERNAL_ERROR` and the message begins `remote timeout`

**E2E-NEW-184** — No new error constant is introduced
- **Category:** Edge · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-049 · **Priority:** High
- When the error code set is enumerated after implementation
- Then it still holds exactly the 14 constants of `crates/mcp-fs/src/errors.rs:9-22`

**E2E-NEW-185** — The fetch response carries every declared field
- **Category:** Feature · **Scenario:** SC-011 · **Requirements:** FR-NEW-050 · **Priority:** Critical
- When a fetch advances `refs/remotes/origin/main` by two commits
- Then the response is `{"refs_updated":[{"ref":"refs/remotes/origin/main","old_sha":"<40 hex>","new_sha":"<40 hex>"}],"refs_stale":[],"objects_fetched":<int>,"up_to_date":false,"auth":"github"}`

**E2E-NEW-186** — `old_sha` is null for a new remote-tracking ref
- **Category:** Edge · **Scenario:** SC-011 · **Requirements:** FR-NEW-050 · **Priority:** High
- When a fetch first creates `refs/remotes/origin/feature-y`
- Then that entry's `old_sha` is null and `new_sha` is the remote tip

**E2E-NEW-187** — `up_to_date` is true exactly when nothing updated
- **Category:** Error · **Scenario:** SC-011 · **Requirements:** FR-NEW-050 · **Priority:** High
- When a fetch runs with no remote change
- Then `refs_updated` is empty and `up_to_date` is true

**E2E-NEW-188** — `git.auth_status` returns the declared shape
- **Category:** Feature · **Scenario:** SC-007 · **Requirements:** FR-NEW-051 · **Priority:** Critical
- Given a valid `github.com` token and an expired `github.ibm.com` token
- When `git.auth_status {}` is called
- Then the response is `{"statuses":[{"host":"github.com",...,"validity":"valid",...},{"host":"github.ibm.com",...,"validity":"expired",...}]}` ordered by host ascending
- And no `authenticated` key appears anywhere

**E2E-NEW-189** — `expires_at` is null for a non-expiring token
- **Category:** Edge · **Scenario:** SC-007 · **Requirements:** FR-NEW-051 · **Priority:** High
- Given a token seeded with no `expires_at`
- When `git.auth_status` is called
- Then that entry's `expires_at` is null and its `validity` is `valid`

**E2E-NEW-190** — Revoking an absent host returns the declared shape
- **Category:** Error · **Scenario:** SC-007 · **Requirements:** FR-NEW-051 · **Priority:** Critical
- When `git.auth_revoke {"host":"git.unknown.test"}` is called
- Then the response is exactly `{"host":"git.unknown.test","provider":null,"revoked":false}`

**E2E-NEW-191** — An RFC 3339 `expires_at` is accepted in any offset
- **Category:** Feature · **Scenario:** SC-002 · **Requirements:** FR-NEW-052 · **Priority:** Critical
- When `git.token_set` is called with `"2027-01-01T00:00:00Z"` and then `"2027-01-01T01:00:00+01:00"`
- Then both store the same instant

**E2E-NEW-192** — A non-RFC-3339 `expires_at` is rejected
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-052 · **Priority:** Critical
- When `expires_at` is `1798761600` or `"tomorrow"`
- Then each is rejected with `ERR_INVALID_ARGUMENT` naming the parameter, and nothing is stored

**E2E-NEW-193** — A past `expires_at` is accepted and reports expired
- **Category:** Edge · **Scenario:** SC-002 · **Requirements:** FR-NEW-052 · **Priority:** High
- When a token is seeded with `expires_at` one hour in the past
- Then seeding succeeds
- And `git.auth_status` reports that host `expired`, and a clone fails per FR-NEW-018

**E2E-NEW-194** — `auth` reports `generic`
- **Category:** Feature · **Scenario:** SC-004 · **Requirements:** FR-NEW-053 · **Priority:** Critical
- When a clone of `https://git.acme.internal/o/r.git` succeeds with a seeded token
- Then the response reports `"auth": "generic"`

**E2E-NEW-195** — `auth` is reported identically by all four operations
- **Category:** Edge · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-053 · **Priority:** High
- When clone, push, fetch and pull each run against `gitlab.acme.corp`
- Then all four report `"auth": "gitlab"`

**E2E-NEW-196** — `anonymous` is never reported as a fallback
- **Category:** Error · **Scenario:** SC-005 · **Requirements:** FR-NEW-053 · **Priority:** Critical
- When an operation targets an undeclared host, and separately a declared host with no token
- Then both fail, and neither response reports `"auth": "anonymous"`

**E2E-NEW-197** — The remote pipeline exists once
- **Category:** Integration · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-054 · **Priority:** Critical
- When the source tree is scanned after implementation
- Then `git2::RemoteCallbacks` is constructed in exactly one file, `crates/mcp-fs/src/git/remote.rs`
- And the entries of `git.hosts` are interpreted in exactly that one file, `config.rs` holding only the field declaration and a single call to `git::remote::validate_hosts` (FR-NEW-065)

**E2E-NEW-198** — A pull charges only the changed bytes
- **Category:** Performance · **Scenario:** SC-012 · **Requirements:** FR-NEW-055 · **Priority:** Critical
- Given a 200 MB tree in which one 10 KB file changed, and `safety.write_quota_bytes` set to 1 MB
- When the pull runs
- Then it succeeds, having charged 10240 bytes, not the tree total

**E2E-NEW-199** — A deleted upstream branch is reported, not pruned
- **Category:** Edge · **Scenario:** SC-011 · **Requirements:** FR-NEW-056 · **Priority:** Critical
- Given `origin/feature-x` was deleted upstream
- When a fetch runs
- Then `refs/remotes/origin/feature-x` still resolves to its previous sha
- And the response contains `"refs_stale":["refs/remotes/origin/feature-x"]`

**E2E-NEW-200** — Each remote operation records one audit entry
- **Category:** Side Effect · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-057 · **Priority:** Critical
- When a push of `main` to `github.ibm.com` succeeds, then a second push is refused as non-fast-forward
- Then two audit entries exist, each naming the tool, host, branch and outcome, the second recording the refusal

**E2E-NEW-201** — Fetch and pull record audit entries too
- **Category:** Error · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-057 · **Priority:** High
- When a fetch succeeds and a pull is refused as dirty
- Then both are recorded, the refusal included

**E2E-NEW-202** — No audit entry carries a credential
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-057 · **Priority:** Critical
- When every remote operation runs with a stored token
- Then no audit entry contains any substring of the token or a userinfo-bearing URL


#### Group N — amendments from audit round 2

**E2E-NEW-203** — The anti-forgery field is rendered with its exact name
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-058 · **Priority:** Critical
- When `GET /app/tokens` is rendered for an authenticated person
- Then the HTML contains `<input type="hidden" name="csrf_token" value="<uuid>">`

**E2E-NEW-204** — A POST carrying `csrf_token` succeeds
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-058 · **Priority:** Critical
- When `POST /app/tokens` submits `host=github.ibm.com&token=ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH&csrf_token=<issued>`
- Then the response is `303` and the token is stored

**E2E-NEW-205** — A missing or consumed `csrf_token` returns the exact error body
- **Category:** Error · **Scenario:** SC-006 · **Requirements:** FR-NEW-058 · **Priority:** Critical
- When the field is omitted, then replayed after being consumed
- Then both return `403` with body `{"error":"ERR_FORBIDDEN","detail":"..."}` and nothing is stored

**E2E-NEW-206** — No response ever sets the cookie
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-059 · **Priority:** Critical
- When every route of the server is exercised, authenticated by cookie and by header
- Then no response carries a `Set-Cookie` header naming `mcpfs_token`

**E2E-NEW-207** — Cross-origin defence does not depend on cookie attributes
- **Category:** Error · **Scenario:** SC-006 · **Requirements:** FR-NEW-059, FR-NEW-058 · **Priority:** Critical
- Given the cookie arrives without any `SameSite` attribute, as an upstream component set it
- When a cross-origin `POST` carries the cookie and no `csrf_token`
- Then it is refused with `403`

**E2E-NEW-208** — The screen works behind a component that sets the cookie
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-059 · **Priority:** High
- Given an upstream component issued `mcpfs_token` and the server never re-issues it
- When the person navigates to `GET /app/tokens`
- Then the screen renders, proving the server needs no cookie-issuing capability

**E2E-NEW-209** — The push response carries all five keys on an update
- **Category:** Feature · **Scenario:** SC-009 · **Requirements:** FR-NEW-060 · **Priority:** Critical
- When a push updates `main`
- Then the response is exactly `{"branch":"main","created":false,"up_to_date":false,"remote_sha":"<40 hex>","auth":"github"}`

**E2E-NEW-210** — The push response carries all five keys when up to date
- **Category:** Error · **Scenario:** SC-009 · **Requirements:** FR-NEW-060 · **Priority:** Critical
- When the same push is repeated with no intervening commit
- Then the response carries the same five keys with `"up_to_date":true`, and no key is absent

**E2E-NEW-211** — `created` and `up_to_date` are never both true
- **Category:** Edge · **Scenario:** SC-009 · **Requirements:** FR-NEW-060 · **Priority:** High
- When a branch is created on the remote, then pushed again unchanged
- Then the first reports `created:true, up_to_date:false` and the second `created:false, up_to_date:true`

**E2E-NEW-212** — A successful push advances the remote-tracking ref
- **Category:** Side Effect · **Scenario:** SC-009 · **Requirements:** FR-NEW-061 · **Priority:** Critical
- When `main` is pushed at sha S
- Then `git.status` lists `refs/remotes/origin/main` at S
- And `refs/heads/main` and every volume file are unchanged

**E2E-NEW-213** — Push creates the remote-tracking ref when absent
- **Category:** Feature · **Scenario:** SC-009 · **Requirements:** FR-NEW-061 · **Priority:** High
- When `feature/x` is pushed for the first time
- Then `refs/remotes/origin/feature/x` exists at the pushed sha

**E2E-NEW-214** — A refused push advances nothing
- **Category:** Error · **Scenario:** SC-010 · **Requirements:** FR-NEW-061 · **Priority:** Critical
- When a push is refused as non-fast-forward
- Then `refs/remotes/origin/main` is exactly where it was before the call

**E2E-NEW-215** — Fetch brings branches under the declared refspec
- **Category:** Feature · **Scenario:** SC-011 · **Requirements:** FR-NEW-062 · **Priority:** Critical
- Given a remote holding branch `main` and tag `v1`
- When a fetch runs
- Then `refs/remotes/origin/main` is updated and appears in `refs_updated`

**E2E-NEW-216** — Fetch takes no tags
- **Category:** Error · **Scenario:** SC-011 · **Requirements:** FR-NEW-062 · **Priority:** Critical
- Given the same remote
- When a fetch runs
- Then `refs/tags/v1` does not exist locally and `git.tags` returns exactly what it returned before

**E2E-NEW-217** — Pull inherits the fetch refspec
- **Category:** Edge · **Scenario:** SC-012 · **Requirements:** FR-NEW-062 · **Priority:** High
- When a pull runs against a remote holding a new tag
- Then no tag is imported and `git.tags` is unchanged

**E2E-NEW-218** — Pulling a branch other than HEAD is refused
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-063 · **Priority:** Critical
- Given `HEAD` points at `refs/heads/main`
- When `git.remote_pull` is called with `{"mount_id":"proj-a","branch":"feature/x"}`
- Then it fails with `ERR_INVALID_ARGUMENT` naming both `feature/x` and `main`
- And no fetch occurred, no ref changed and no file changed

**E2E-NEW-219** — Pulling the checked-out branch proceeds
- **Category:** Feature · **Scenario:** SC-012 · **Requirements:** FR-NEW-063 · **Priority:** Critical
- Given `HEAD` points at `refs/heads/main`
- When the pull names `main`
- Then it proceeds normally

**E2E-NEW-220** — The branch check runs before the network
- **Category:** Edge · **Scenario:** SC-012 · **Requirements:** FR-NEW-063 · **Priority:** High
- Given a connection recorder and a volume whose `HEAD` is `main`
- When a pull names `feature/x`
- Then zero outbound connections were attempted

**E2E-NEW-221** — A failed persistence write leaves no in-memory token
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-064 · **Priority:** Critical
- Given an empty store and persistence made unavailable
- When `git.token_set` is called for `github.ibm.com` and fails
- Then `git.auth_status` lists no entry for that host
- And a clone of that host fails per EXC-004a rather than succeeding

**E2E-NEW-222** — A failed write restores the previous token
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-064 · **Priority:** Critical
- Given a valid token already stored for `github.ibm.com`
- When a re-seed fails at the persistence write
- Then the previous token is still present and still authenticates a clone

**E2E-NEW-223** — The device-flow poller rolls back identically
- **Category:** Edge · **Scenario:** SC-003 · **Requirements:** FR-NEW-064 · **Priority:** High
- When the poller authorises successfully but its persistence write fails
- Then no in-memory token remains for that `(person, host)`

**E2E-NEW-224** — Validation and resolution live in one module
- **Category:** Integration · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-065 · **Priority:** Critical
- When the source tree is scanned after implementation
- Then `git::remote::validate_hosts` and `git::remote::resolve_host` exist in `crates/mcp-fs/src/git/remote.rs`
- And `config.rs` contains the `hosts` field declaration and exactly one call to `validate_hosts`, and no other inspection of an entry's value

**E2E-NEW-225** — Boot validation still fires from `ServerConfig::validate`
- **Category:** Feature · **Scenario:** SC-001 · **Requirements:** FR-NEW-065 · **Priority:** Critical
- Given a map with an unknown provider value
- When the server starts
- Then boot fails, proving `validate_hosts` is reached from `ServerConfig::validate`

**E2E-NEW-226** — A valid map passes validation from the same path
- **Category:** Edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-065 · **Priority:** High
- Given the reference map
- When the server starts
- Then it boots and every host resolves through `git::remote::resolve_host`

**E2E-NEW-227** — `instance_url` supplies the host when host is omitted
- **Category:** Feature · **Scenario:** SC-003 · **Requirements:** FR-NEW-066 · **Priority:** Critical
- Given `gitlab.acme.corp: gitlab` declared
- When `git.auth` is called with `{"provider":"gitlab","instance_url":"https://gitlab.acme.corp"}` and the flow succeeds
- Then the token is stored under `(alice@test.com, gitlab.acme.corp)`
- And nothing is stored under `gitlab.com`

**E2E-NEW-228** — A disagreeing `instance_url` and `host` are rejected
- **Category:** Error · **Scenario:** SC-003 · **Requirements:** FR-NEW-066 · **Priority:** Critical
- When `git.auth` is called with `{"provider":"gitlab","instance_url":"https://gitlab.acme.corp","host":"gitlab.com"}`
- Then it fails with `ERR_INVALID_ARGUMENT` naming both `gitlab.acme.corp` and `gitlab.com`
- And no device code is requested

**E2E-NEW-229** — The canonical default applies only when both are absent
- **Category:** Edge · **Scenario:** SC-003 · **Requirements:** FR-NEW-066 · **Priority:** High
- When `git.auth` is called with `{"provider":"gitlab"}` alone
- Then the token is stored under `gitlab.com`, and the stored `instance_url` is null

**E2E-NEW-230** — The screen orders rows by host
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-067 · **Priority:** High
- Given tokens for `github.ibm.com` and `github.com`
- When the screen renders
- Then `github.com` appears before `github.ibm.com`

**E2E-NEW-231** — The screen and the tool agree on order
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-067 · **Priority:** High
- Given tokens for four hosts
- When the screen and `git.auth_status` are compared
- Then the host sequences are identical

**E2E-NEW-232** — An empty token list renders without error
- **Category:** Error · **Scenario:** SC-006 · **Requirements:** FR-NEW-067 · **Priority:** High
- Given a person holding no tokens
- When the screen renders
- Then it returns `200` with an empty list and no ordering failure


#### Group O — amendments from audit round 3

**E2E-NEW-233** — Revoke by provider alone targets the canonical host
- **Category:** Feature · **Scenario:** SC-007 · **Requirements:** FR-NEW-068 · **Priority:** Critical
- Given tokens for `github.com` and `github.ibm.com`
- When `git.auth_revoke {"provider":"github"}` is called
- Then the response is `{"host":"github.com","provider":"github","revoked":true}` and `github.ibm.com` remains

**E2E-NEW-234** — Revoke by host alone succeeds without a provider
- **Category:** Feature · **Scenario:** SC-007 · **Requirements:** FR-NEW-068 · **Priority:** Critical
- When `git.auth_revoke {"host":"github.ibm.com"}` is called with no `provider`
- Then it succeeds with `{"host":"github.ibm.com","provider":"github","revoked":true}`

**E2E-NEW-235** — Revoke with neither parameter is rejected
- **Category:** Error · **Scenario:** SC-007 · **Requirements:** FR-NEW-068 · **Priority:** Critical
- When `git.auth_revoke {}` is called
- Then it fails with `ERR_INVALID_ARGUMENT` naming both `provider` and `host`

**E2E-NEW-236** — Revoke with a disagreeing provider and host is rejected
- **Category:** Error · **Scenario:** SC-007 · **Requirements:** FR-NEW-068 · **Priority:** High
- When `git.auth_revoke {"provider":"gitlab","host":"github.ibm.com"}` is called
- Then it fails with `ERR_INVALID_ARGUMENT` naming both values

**E2E-NEW-237** — Revoking an undeclared host is possible and reports a null provider
- **Category:** Edge · **Scenario:** SC-007 · **Requirements:** FR-NEW-068 · **Priority:** High
- When `git.auth_revoke {"host":"git.unknown.test"}` is called
- Then it returns `{"host":"git.unknown.test","provider":null,"revoked":false}` rather than an undeclared-host error

**E2E-NEW-238** — A pull charges only the blobs it writes
- **Category:** Performance · **Scenario:** SC-012 · **Requirements:** FR-NEW-069 · **Priority:** Critical
- Given a 200 MB tree in which one 10 KB file changed, and `safety.write_quota_bytes` of 1 MB
- When the pull runs
- Then it succeeds, having charged 10240 bytes

**E2E-NEW-239** — A merge charges only its changed blobs
- **Category:** Error · **Scenario:** SC-015 · **Requirements:** FR-NEW-069 · **Priority:** Critical
- Given a merged tree whose changed blobs sum to 4 MB and a quota of 1 MB
- When the pull runs with `on_conflict`
- Then it is refused, no merge commit is created and `refs/heads/main` is unchanged

**E2E-NEW-240** — An unchanged path is charged nothing
- **Category:** Edge · **Scenario:** SC-012 · **Requirements:** FR-NEW-069 · **Priority:** High
- Given a fast-forward whose tree is byte-identical to the current volume
- When the pull runs with a quota of 0 bytes
- Then it succeeds, having charged nothing

**E2E-NEW-241** — A missing token yields the fixed code and prefix on all four tools
- **Category:** Error · **Scenario:** SC-004 · **Requirements:** FR-NEW-070 · **Priority:** Critical
- Given `github.ibm.com` declared `github` with no stored token
- When clone, push, fetch and pull each run
- Then all four fail with `ERR_UNAUTHENTICATED` and a message beginning `no token for host github.ibm.com`
- And no socket is opened in any of the four

**E2E-NEW-242** — An expired token yields its own fixed prefix
- **Category:** Error · **Scenario:** SC-014 · **Requirements:** FR-NEW-070 · **Priority:** Critical
- Given a token for that host expired one hour ago
- When the same four operations run
- Then all four fail with `ERR_UNAUTHENTICATED` and a message beginning `token expired for host github.ibm.com`

**E2E-NEW-243** — Absence and expiry are machine-distinguishable
- **Category:** Edge · **Scenario:** SC-014 · **Requirements:** FR-NEW-070 · **Priority:** Critical
- When one operation runs with no token and another with an expired one
- Then both carry `ERR_UNAUTHENTICATED` and their message prefixes differ, so a client can choose between seeding and authenticating

**E2E-NEW-244** — A push without `branch` is rejected
- **Category:** Error · **Scenario:** SC-009 · **Requirements:** FR-NEW-071 · **Priority:** Critical
- When `git.remote_push {"mount_id":"proj-a"}` is called
- Then it fails with `ERR_INVALID_ARGUMENT` naming `branch`, with no default to the checked-out branch

**E2E-NEW-245** — A pull without `branch` is rejected
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-071 · **Priority:** Critical
- When `git.remote_pull {"mount_id":"proj-a"}` is called
- Then it fails with `ERR_INVALID_ARGUMENT` naming `branch`

**E2E-NEW-246** — The optional parameters are genuinely optional
- **Category:** Feature · **Scenario:** SC-002 · **Requirements:** FR-NEW-071 · **Priority:** Critical
- When `git.token_set` is called without `expires_at` and `git.remote_fetch` with only `mount_id`
- Then both succeed
- And the regenerated contract records exactly the required lists FR-NEW-071 declares

**E2E-NEW-247** — Every remote operation emits exactly one span
- **Category:** Side Effect · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-072 · **Priority:** Critical
- Given a tracing subscriber capturing spans
- When a push succeeds and a clone is refused for an undeclared host
- Then exactly two `git.remote` spans exist, carrying `operation`, `host`, `provider`, `outcome` and `duration_ms`, the second with `outcome=error`
- And neither span contains a token, an `Authorization` value or URL userinfo

### 12.3 Modified Test Specifications

Twenty-six existing tests change. Each keeps its assertion and gains the new key or schema; the
full list with file and line is in Section 9.3. Three deserve explicit specification because
their assertion changes, not merely their inputs.

**E2E-MOD-001** — `providers_are_independent` becomes `hosts_are_independent` (`crates/mcp-fs/src/git/oauth/store.rs:234`)
- **Original test:** proved a `github` token and a `gitlab` token for one person are independent.
- **Modified to validate:** that two tokens for the **same provider on different hosts** are independent, which the old key made impossible.
- **Steps:**
  - Given tokens stored for `(alice@test.com, github.com)` and `(alice@test.com, github.ibm.com)`
  - When each is retrieved
  - Then the two values differ and neither overwrote the other
  - And revoking one leaves the other intact

**E2E-MOD-002** — `an_expired_token_reports_as_unauthenticated` (`crates/mcp-fs/src/tools/git_auth.rs:526`)
- **Original test:** an expired token made `git.auth_status` report unauthenticated.
- **Modified to validate:** the host is reported with validity `expired`, distinct from being absent, per FR-NEW-019.
- **Steps:**
  - Given an expired token for `(alice@test.com, github.ibm.com)`
  - When `git.auth_status` is called
  - Then `github.ibm.com` is listed with validity `expired`
  - And a host with no token is not listed at all

**E2E-MOD-003** — `revoke_clears_the_token_and_is_idempotent` (`crates/mcp-fs/src/tools/git_auth.rs:545`)
- **Original test:** revoke removed the provider's token and repeated calls succeeded.
- **Modified to validate:** revoke removes exactly one host and provably leaves every other host untouched.
- **Steps:**
  - Given tokens for `github.com` and `github.ibm.com`
  - When `git.auth_revoke` is called for `github.ibm.com` twice
  - Then both calls succeed and `github.com` still authenticates a clone

**E2E-MOD-004** — `gitlab_keeps_the_instance_url_on_the_stored_session` (`crates/mcp-fs/src/tools/git_auth.rs:454`)
- **Original test:** proved a GitLab device flow kept `instance_url` on the stored session.
- **Modified to validate:** the session keys on the host derived from `instance_url`, and still carries `instance_url`.
- **Steps:**
  - Given `gitlab.acme.corp: gitlab` is declared
  - When `git.auth` is called with `{"provider":"gitlab","instance_url":"https://gitlab.acme.corp"}` and the fake flow succeeds
  - Then the token is stored under `(alice@test.com, gitlab.acme.corp)`
  - And its `instance_url` is `https://gitlab.acme.corp`
  - And nothing is stored under `gitlab.com`

### 12.4 Removed Tests

None. No behaviour is deleted by this specification; every affected test survives with an
updated key or schema.

---

## 13. Consistency Notes

`specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md` is **not modified**. Three deviations from it are recorded
here.

**13.1 `FR-624` (device authorization) gains a host dimension.** The original describes
authorization per provider. FR-MOD-002 adds an optional `host`, defaulting to the canonical
public host so the original's behaviour is preserved exactly when `host` is omitted.
**Resolution:** additive; no existing caller breaks.

**13.2 `FR-629` (authorization status and revocation) becomes per host.** The original reports
and revokes per provider. FR-MOD-003 and FR-MOD-004 make both per host. Revocation is where the
semantics genuinely change: under the original, revoking `github` removed the person's only
GitHub token; under FR-MOD-004 it removes only the canonical host's token. **Resolution:**
deliberate, decided at interview (DEC-029). Revoking across hosts would destroy a credential the
caller never named.

**13.3 Anonymous clone behaviour is deliberately broken.** The git spec's clone behaviour, as
implemented at `crates/mcp-fs/src/tools/git.rs:693-706`, falls back to `auth: "anonymous"`
whenever no provider is detected. FR-NEW-007 makes an undeclared host an error.
**Resolution:** accepted explicitly (DEC-010) on the grounds that this deployment rarely clones
public repositories and that implicit anonymity was the mechanism by which the GHES defect went
unnoticed. Anonymous access remains available and must be declared.

**No inconsistency was found that the user did not adjudicate.** Each of the three is recorded
as a decision in Section 17.

---

## 14. Migration & Implementation Notes

### 14.1 Required implementation order

These orderings are load-bearing: reversing any pair leaves the build or the suite broken
between steps.

1. **Config and host map before every consumer.** FR-NEW-001 to FR-NEW-006 first. FR-MOD-001,
   FR-NEW-007 and FR-NEW-008 all read the map; writing them first means writing against a type
   that does not exist.
2. **Token key change before the seeding tool.** FR-NEW-009 and FR-NEW-010 before FR-NEW-013.
   `git.token_set` calls `store_token`, whose signature changes. Building the tool first means
   building it twice.
3. **Token key change before the auth tool modifications.** FR-NEW-009 before FR-MOD-002,
   FR-MOD-003 and FR-MOD-004, for the same reason.
4. **Schema migration alongside the key change, never after.** FR-NEW-010 and FR-NEW-011 land in
   the same change as FR-NEW-009. A server whose in-memory key is `(person, host)` and whose
   table key is `(person, provider)` cannot load its own persisted state.
5. **`git::remote` extraction before the three new tools.** DEC-034's module must exist, with
   clone migrated onto it and green, before push, fetch and pull are added. Adding them first
   creates the four duplicate pipelines the approach exists to prevent.
6. **`origin` persistence before push, fetch and pull.** FR-NEW-020 before FR-NEW-021 to
   FR-NEW-031. The three tools have no other source for the URL, so without it every one of
   their tests can only assert the no-origin error.
7. **Fetch before pull.** FR-NEW-026 and FR-NEW-027 before FR-NEW-028. Pull's first step is a
   fetch.
8. **Fast-forward pull before merge.** FR-NEW-028 before FR-NEW-031. Merge is the fallback when
   the fast-forward test fails; the test must exist first.
9. **Contract regeneration last.** FR-NEW-045 after every schema is final. Regenerating early
   means regenerating repeatedly, and each regeneration is a reviewed diff.

### 14.2 Operator upgrade procedure

1. Announce that every stored git token will be dropped and each person re-authenticates once.
2. Add `git.hosts`, declaring every host in use, including any public host that must stay
   anonymous. **A host omitted here stops working**, by design.
3. Deploy. Boot fails loudly on an invalid map rather than degrading.
4. Verify with `git.auth_status`: every person reports zero tokens.
5. People re-authenticate with `git.auth`, or callers reseed with `git.token_set`.

### 14.3 Rollback

Reverting the code does not restore dropped tokens; a rollback costs a second re-authentication.
The change is reversible in code and not in data. Operators are told this before upgrading.

---

## 15. Open Questions & TBDs

1. **`expires_at` representation.** FR-NEW-014 requires a non-expiring token. The existing
   column is non-null. Whether the implementation relaxes the column or adopts a far-future
   sentinel is the implementer's choice; the observable requirement is that `git.auth_status`
   reports such a token `valid` indefinitely and FR-NEW-018 never fires for it. **TBD:
   representation only, not behaviour.**
2. **Dirty-volume detection cost.** FR-NEW-029 requires comparing the volume against the branch
   tip. On a very large volume this is a full tree walk on every pull. No performance budget was
   set at interview. **TBD: whether a cheaper signal is needed; it is a pull-time cost only.**
3. **Token screen route path.** ~~Previously open.~~ **Resolved** by FR-NEW-046, which fixes
   `GET /app/tokens`, `POST /app/tokens` and `POST /app/tokens/revoke`, and by FR-NEW-047 and
   FR-NEW-048, which fix how identity reaches those routes and how cross-origin submissions are
   refused.
4. **Punycode handling.** E2E-NEW-062 requires deterministic IDN behaviour and assumes resolution
   on the punycode form. **TBD: confirm the `url` crate's normalisation matches that assumption.**

---

## 16. Glossary

| Term | Definition | Context |
|---|---|---|
| **Host** | The bare hostname of a remote, extracted by parsing a URL. Never a URL, never a substring. | Host Resolution |
| **Provider** | A credential policy: `github`, `gitlab`, `generic` or `anonymous`. Decided by Host Resolution, recorded as an attribute by Token Custody. | Both |
| **Host map** | The `git.hosts` configuration mapping hosts to providers. Boot-validated, immutable at runtime, authoritative. | Host Resolution |
| **Declared host** | A host present in the host map. Its absence is an error, never an implicit anonymous. | Host Resolution |
| **Canonical public host** | The host assumed when `host` is omitted: `github.com` for `github`, `gitlab.com` for `gitlab`. | Token Custody |
| **Seeding** | Supplying an already-held token to the server directly, without an interactive flow. | Token Custody |
| **Token identity** | The pair `(person, host)` that uniquely identifies one stored credential. | Token Custody |
| **Device flow** | The OAuth device authorization grant, the pre-existing way to obtain a token interactively. | Token Custody |
| **Origin** | The single remote recorded for a volume at clone time, in `git_remotes` under the name `origin`. | Remote Operations |
| **Fast-forward** | An advance where the current tip is an ancestor of the target, requiring no merge. | Remote Operations |
| **Divergence** | The state where neither the local tip nor the remote tip is an ancestor of the other. | Remote Operations |
| **Conflict strategy** | The global `ours` or `theirs` choice resolving every conflicting file in one merge. | Remote Operations |
| **Dirty volume** | A volume whose files differ from its current branch tip. There is no staging area; the volume is the working tree. | Remote Operations |
| **Volume** | A project's simulated filesystem, which is simultaneously the git working tree. | All |
| **Person** | An authenticated human identity, the owner of tokens and the subject of authorization. | All |
| **Platform admin** | An identity managing projects and membership, with no implicit file or token access. | Token Self-Service |

---

## 17. Interview Decisions Log

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-002:** Providers stay `github` and `gitlab`, plus an explicit `generic`. No hardcoded host values. **Rationale:** GHES genuinely is provider `github`, so the credential shape and `git.auth`'s validation stay correct; `generic` covers self-hosted hosts without opening the set to arbitrary strings. **Alternatives considered:** adding `azure` with its own credential shape; a fully open config-defined set. **Implemented by:** FR-NEW-001. **Round:** 1. **Code evidence:** `crates/mcp-fs/src/tools/git_auth.rs:160-162`
- **DEC-003:** Token identity becomes per host, not per provider. **Rationale:** a person may hold distinct tokens for `github.com` and `github.ibm.com`, both provider `github`. **Implemented by:** FR-NEW-009, FR-NEW-010. **Round:** 1. **Code evidence:** `crates/mcp-fs/src/git/oauth/store.rs:65-67`, `crates/mcp-fs/src/git/oauth/persistence.rs:42`
- **DEC-004:** The key is `(person, host)`; provider is an attribute of the row. **Alternatives considered:** `(person, provider, host)`, which permits two tokens per host with no known use. **Implemented by:** FR-NEW-009. **Round:** 1. **Code evidence:** n/a
- **DEC-005:** Pre-existing `oauth_tokens` rows are dropped at upgrade, not migrated. **Rationale:** the old rows carry no host, and inferring one from `provider` writes a guess into a primary key. **Alternatives considered:** inferring `github.com`/`gitlab.com`; reading the existing `instance_url` column. **Implemented by:** FR-NEW-011. **Round:** 1. **Code evidence:** `crates/mcp-fs/src/git/oauth/persistence.rs:42`
- **DEC-006:** The three auth tools gain an optional `host`. **Rationale:** with a per-host key, a provider-only revoke is ambiguous. **Implemented by:** FR-MOD-002, FR-MOD-003, FR-MOD-004. **Round:** 1. **Code evidence:** `crates/mcp-fs/src/tools/git_auth.rs:76-82`, `:104-116`, `:126-131`
- **DEC-007:** *(Superseded by DEC-028.)* The screen was to be a single tool returning a URL for a self-expiring session. **Round:** 1.
- **DEC-008:** The screen reads and writes but never displays a token value; it lists hosts, seeds and revokes. **Implemented by:** FR-NEW-038, FR-NEW-039. **Round:** 1. **Code evidence:** n/a
- **DEC-009:** Non-goals: refresh/rotation, providers beyond the four, SSH, runtime map editing, and changing the `oauth2:<token>` credential shape. Revocation is in scope. **Rationale:** the token is consumed as an HTTPS password and already works for read and write, so there is no complexity to add. **Implemented by:** n/a — scope boundary, Section 3.2. **Round:** 1. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1014`
- **DEC-010:** An undeclared host is a configuration error, not a silent downgrade to anonymous. **Rationale:** explicit over implicit; public repositories are the exception here. **Accepted trade:** breaks clones of public repositories from undeclared hosts that work today. **Implemented by:** FR-NEW-007, FR-NEW-008. **Round:** 2a. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:683-706`
- **DEC-011:** Scope grows to full bidirectional remote sync. **Rationale:** a token that can only clone cannot serve the write path. **Alternatives considered:** push only; deferring both. **Implemented by:** FR-NEW-021 to FR-NEW-036. **Round:** 2a. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1009` is the only `RemoteCallbacks` in the tree
- **DEC-012:** Both `git.remote_fetch` and `git.remote_pull`, with pull refusing anything that is not a fast-forward unless a strategy is given. **Rationale:** the volume is the working tree, so unconstrained merging would need conflict representation in a simulated filesystem. **Implemented by:** FR-NEW-026 to FR-NEW-031. **Round:** 2a. **Code evidence:** n/a
- **DEC-013:** Push is fast-forward only; no force. **Implemented by:** FR-NEW-024. **Round:** 2a. **Code evidence:** n/a
- **DEC-014:** Push takes an explicit `branch` and creates it on the remote when absent. **Implemented by:** FR-NEW-022, FR-NEW-023. **Round:** 2a. **Code evidence:** n/a
- **DEC-015:** Host matching is exact only; no wildcards. **Rationale:** wildcards reintroduce the precedence ambiguity substring matching caused. **Implemented by:** FR-NEW-004, FR-NEW-006. **Round:** 2b. **Code evidence:** n/a
- **DEC-016:** The seeding tool is named `git.token_set`. **Implemented by:** FR-NEW-013. **Round:** 2b. **Code evidence:** n/a
- **DEC-017:** `git.token_set` takes an optional `expires_at`; absent means non-expiring. **Rationale:** a PAT may genuinely never expire, and a fabricated expiry would wrongly trigger the expiry failure path. **Implemented by:** FR-NEW-014. **Round:** 2b. **Code evidence:** `crates/mcp-fs/src/git/oauth/store.rs:115-123`
- **DEC-018:** An omitted `host` on `git.auth` defaults to the provider's canonical public host. **Rationale:** every existing caller keeps working unchanged. **Implemented by:** FR-MOD-002. **Round:** 2b. **Code evidence:** n/a
- **DEC-019:** An expired stored token is treated as absent-with-a-reason; operations fail fast telling the person to re-authenticate. **Implemented by:** FR-NEW-018. **Round:** 2b. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:704`, `crates/mcp-fs/src/git/oauth/store.rs:148-150`, `:169`
- **DEC-020:** An expired token is left in the store, not deleted on detection. **Rationale:** lets status report `expired` rather than absent; deletion on a read path is a surprising side effect. **Implemented by:** FR-NEW-019. **Round:** 2b. **Code evidence:** n/a
- **DEC-021:** Push, fetch and pull take no `url`; they resolve the stored `origin`. No remote-management tools. **Implemented by:** FR-NEW-021. **Round:** 2b. **Code evidence:** `crates/mcp-fs/src/git/db.rs:298`
- **DEC-022:** `git.remote_clone` must persist the clone URL as `origin`. **Rationale:** push, fetch and pull have no other source for the URL, and the table plus its API already exist unused. **Implemented by:** FR-NEW-020. **Round:** 2b. **Code evidence:** table `crates/mcp-fs/src/git/db.rs:69-75`, API `:281-305`, test `:429-442`, and no call anywhere in `crates/mcp-fs/src/tools/git.rs:715-830`
- **DEC-023:** Pull is atomic; a failed file write leaves the ref unadvanced. **Rationale:** unlike clone, which is recoverable by re-cloning, a pull that advances the ref over stale files leaves the volume silently inconsistent with its own HEAD. **Implemented by:** FR-NEW-035. **Round:** 2b. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:785-791`
- **DEC-024:** `git.remote_pull` takes an optional `on_conflict` of `ours` or `theirs`; absent refuses a non-fast-forward, present runs a three-way merge with that global strategy. **Rationale:** closes the divergence dead end without interactive resolution. **Alternatives considered:** documenting the dead end; adding `git.discard_changes`; allowing a hard reset that destroys local commits. **Implemented by:** FR-NEW-030, FR-NEW-031, FR-NEW-032, FR-NEW-033. **Round:** 2b. **Code evidence:** `git2-0.20.4/src/merge.rs:133-136`, `git2-0.20.4/src/repo.rs:2177`
- **DEC-025:** A dirty volume still refuses the pull; the "commit or discard" guidance is now true because committing no longer strands the volume. **Implemented by:** FR-NEW-029. **Round:** 2b. **Code evidence:** n/a
- **DEC-026:** The merge commit is authored by the authenticated person. **Implemented by:** FR-NEW-034. **Round:** 2b. **Code evidence:** n/a
- **DEC-027:** The merge commit message is auto-generated and records the strategy; no override. **Rationale:** the resolution stays visible in `git.log` permanently. **Implemented by:** FR-NEW-034. **Round:** 2b. **Code evidence:** n/a
- **DEC-028:** The token screen is served from the main server behind the RS256 identity layer, superseding DEC-007. **Rationale:** the `doc.open_editor` precedent binds loopback with no token in the URL, so it is unauthenticated against other local processes and unreachable when the server is remote. **Alternatives considered:** mirroring the editor exactly; hardening the spawned session with a URL token and TTL. **Implemented by:** FR-NEW-037, FR-NEW-040. **Round:** 2b. **Code evidence:** `crates/mcp-fs/src/tools/editor.rs:208`, `:216`, `:293`
- **DEC-029:** `git.auth_revoke` never revokes across hosts. **Rationale:** `github.com` and `github.ibm.com` are different hosts holding different tokens. **Implemented by:** FR-MOD-004. **Round:** 2b. **Code evidence:** n/a
- **DEC-030:** `git.auth_revoke` with `host` omitted defaults to the canonical public host, uniform with DEC-018. **Alternatives considered:** making `host` required, a breaking schema change; revoking all hosts for the provider. **Implemented by:** FR-MOD-004. **Round:** 2b. **Code evidence:** n/a
- **DEC-031:** A single spec; `/plan-spec` partitions it into stories. **Rationale:** the groups are coupled — push, fetch and pull are inert without the keying and mapping work. **Alternatives considered:** splitting into a foundation spec that would unblock Graph Studio sooner plus a remote-sync spec. **Implemented by:** n/a — process decision. **Round:** 3. **Code evidence:** n/a
- **DEC-032:** `git.remote_timeout_secs`, configurable, default 120. **Implemented by:** FR-NEW-044. **Round:** 4. **Code evidence:** n/a
- **DEC-033:** Seeded tokens longer than 8192 characters are rejected. **Rationale:** a denial-of-service bound; the codebase already uses bounded `TextKey` columns because SQL Server cannot index unbounded text. **Implemented by:** FR-NEW-015. **Round:** 4. **Code evidence:** `crates/mcp-fs/src/git/oauth/persistence.rs:33-34`
- **DEC-034:** Approach B — extract `crates/mcp-fs/src/git/remote.rs` owning host resolution, credential supply and the four remote operations; `tools/git.rs` becomes a thin adapter. **Rationale:** the security properties are properties of the shared pipeline; one implementation means one set of proofs rather than four. **Alternatives considered:** extending in place, leaving four call sites to re-verify per security property; a `CredentialProvider` trait, speculative while the credential shape is frozen. **Implemented by:** Section 14.1 step 5, and every FR in groups F to K. **Round:** 2 (approach exploration). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:683-706`, `:1003-1020`
- **DEC-036:** A pull charges against the write quota only the bytes of files whose path or content actually changes, not the whole target tree. **Rationale:** charging a 200 MB tree for a 10 KB change would refuse ordinary pulls on large repositories. **Alternatives considered:** charging the whole tree, as clone does, where every file genuinely is new. **Implemented by:** FR-NEW-055. **Round:** 6 (audit round 1, finding F-009). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:778-782`
- **DEC-037:** Fetch does not prune remote-tracking refs for branches deleted upstream; it reports them in `refs_stale`. **Rationale:** pruning is a destructive ref operation nobody asked for. **Alternatives considered:** pruning silently, as `git fetch --prune` does. Pruning is recorded in `specs/BACKLOG.md` under the standard-git divergence review. **Implemented by:** FR-NEW-056. **Round:** 6 (audit round 1, finding F-010). **Code evidence:** n/a
- **DEC-038:** The token screen accepts a read-only `mcpfs_token` cookie in addition to the two bearer headers, with cross-origin submissions refused by a single-use `csrf_token`. **Rationale:** a browser cannot attach an `Authorization` header to a top-level navigation, so a header-only screen is unreachable by the only client that can use it. **Alternatives considered:** requiring an authenticating gateway to inject the forwarded header, which adds no credential surface but makes the screen unusable without a gateway; a capability URL, rejected because it puts a credential in a URL, which FR-NEW-042 forbids elsewhere in this same specification. **Implemented by:** FR-NEW-047, FR-NEW-048, FR-NEW-058, FR-NEW-059. **Round:** 6 (audit round 1, finding F-002). **Code evidence:** `crates/mcp-fs/src/identity.rs:1-8`, `:188-193`
- **DEC-035:** The test plan was authored by the same agent that authored the requirements, not by an independent test-designer sub-agent as depth L mandates. **Rationale:** none — this is a deviation, not a choice. The Agent tool returned HTTP 403 on every permitted model across three attempts. **Consequence:** the author-designs-own-tests bias this step exists to remove is present. **Implemented by:** n/a — process deviation. **Round:** 4. **Code evidence:** n/a

---

## 18. Implementability Audit

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|---|---|---|---|
| 1 | 13 | 10 | NOT-IMPLEMENTABLE |
| 2 | 10 | 2 | NOT-IMPLEMENTABLE |
| 3 | 5 | 1 | NOT-IMPLEMENTABLE |

Three rounds were run, each by an independent auditor receiving the spec path and the audit
contract and nothing else. Every round verified each cited `file:line`, recounted every asserted
count, and confirmed every announced test identifier was specified.

**No finding ever survived a round.** Round 2 re-reported none of round 1's thirteen; round 3
re-reported none of round 2's ten. The F count fell 13, 10, 5. Every round found *different*
gaps, which is the documented signal that more rounds will keep finding one more thing.

**Round 1 amendments:** FR-NEW-046 to FR-NEW-057; FR-NEW-045 corrected from five new tools to
four; coverage statistics recounted from the tests themselves; 39 tests added. Three product
choices were decided by the user: DEC-036, DEC-037, DEC-038.

**Round 2 amendments:** FR-NEW-058 to FR-NEW-067; the `Set-Cookie` contradiction between
FR-NEW-047 and FR-NEW-048 resolved; E2E-NEW-197's "one file reads `git.hosts`" clause restated,
since boot validation must fire from `ServerConfig::validate`; 30 tests added; E2E-MOD-004 added.

**Round 3 amendments:** FR-NEW-068 to FR-NEW-072; the write-quota contradiction between
FR-NEW-036 and FR-NEW-055 resolved in favour of the delta, and E2E-NEW-133 and E2E-NEW-141
restated so they no longer contradict E2E-NEW-198; 15 tests added.

**Status: the three-round limit is reached and round 3's five F findings were amended but NOT
re-audited.** Per the governing process a fourth round is not run and the specification is not
committed on the authority of this audit. The decision is escalated to the user. What is known:
every F finding raised across three rounds has an amendment applied in the file; no amendment
has been independently verified since round 3; the convergence pattern, all-new findings each
round, indicates residual naming and response-shape gaps of the same class rather than
functional gaps in the interview's coverage.

**Drift registered:** DRIFT-003, DRIFT-004, DRIFT-005, DRIFT-008, DRIFT-009, DRIFT-010.
Evident corrections applied in place instead of registered: incorrect `git_auth.rs` and
`tools/git.rs` line citations; the non-existent error names `ERR_PERMISSION_DENIED` and
`ERR_INTERNAL`; the false claim that the repository has no `tests/` directory; the false claim
that `git2` is pinned rather than a caret range; the functional scenario count, 15 not 16; and
the three decisions DEC-036, DEC-037 and DEC-038 that were cited before being recorded.

## 19. Implementation Drift Register

> Spec/code alignment drift found at audit. The behaviour specified is correct; the statement
> about the current code was not, or the capability the requirement needs does not exist yet.
> **These are resolved DURING the implementation of this spec, not later.** Every entry must be
> closed before the implementation branch merges. An entry still open at merge time moves to
> `specs/BACKLOG.md` with its evidence, and that move is a decision someone signs, not a silence.

Four further A findings from round 1 were evident corrections and were applied in place rather
than registered: incorrect `git_auth.rs` line citations, incorrect `tools/git.rs` line citations,
the non-existent error names `ERR_PERMISSION_DENIED` and `ERR_INTERNAL`, and the false claim that
the repository has no `tests/` directory.

#### DRIFT-003: The `url` crate is not a direct dependency
- **Spec says:** "No new external dependencies... `url` parsing is already in the tree" (§9.5, as originally written). FR-NEW-006 requires the hostname to be obtained by parsing the URL.
- **Code does:** `url` appears in `Cargo.lock` as a transitive dependency only. It is absent from `Cargo.toml` `[workspace.dependencies]` and from `crates/mcp-fs/Cargo.toml` `[dependencies]`, and no `url::` reference exists anywhere in `crates/mcp-fs/src`.
- **Nature:** missing capability
- **Resolution during implementation:** Add `url = "2.5"` to `[workspace.dependencies]` and `url = { workspace = true }` to `crates/mcp-fs/Cargo.toml`. Section 9.5 has already been corrected to state one new direct dependency.
- **Detected by:** `cargo build` fails with `error[E0432]: unresolved import url`.
- **Blocks which requirement:** FR-NEW-006, and through it FR-NEW-007, FR-NEW-041, FR-NEW-042.
- **Status:** open

#### DRIFT-004: The `migrate` verb does not copy `oauth_tokens` at all
- **Spec says:** §9.1 lists `crates/mcp-fs/src/migrate.rs` as a **Moderate** impact that "carries the new `oauth_tokens` key". FR-NEW-012 requires the verb to copy the table under the new key.
- **Code does:** `migrate()` copies `project` and `project_member` (`crates/mcp-fs/src/migrate.rs:200-212`), `nodes` and `blob_refs` (`:231-247`), and `crate::git::db::TABLES` (`:273-277`). `oauth_tokens` is never opened and never copied; `crates/mcp-fs/src/git/oauth/persistence.rs:28` `schema()` has no caller in `migrate.rs`.
- **Nature:** missing capability
- **Resolution during implementation:** FR-NEW-012 is new behaviour, not a re-key. Open the source and destination oauth stores, apply `git::oauth::persistence::schema()` to both, and copy the table using the same row-count check as `migrate.rs:204-211`. Correct the §9.1 impact from "Moderate" to "New behaviour".
- **Detected by:** E2E-NEW-041 fails, because the destination holds zero rows.
- **Blocks which requirement:** FR-NEW-012.
- **Status:** open

#### DRIFT-005: The relational schema layer cannot change a primary key
- **Spec says:** FR-NEW-010 requires `oauth_tokens` to be keyed `(person, host)`; FR-NEW-011 requires every pre-existing row to be dropped and the table rebuilt.
- **Code does:** Migration renders only `CREATE TABLE IF NOT EXISTS` (`crates/mcp-fs/src/storage/rel/schema.rs:237-241`) and `ALTER TABLE ... ADD COLUMN` (`crates/mcp-fs/src/storage/rel/dialect.rs:399-420`), a limit stated outright at `schema.rs:115-116`. There is no `DROP TABLE` renderer and no primary-key migration in any dialect. Applied against an existing table, the new `SchemaSet` is a **silent no-op**: the old `(person, provider)` key and the missing `host` column both survive.
- **Nature:** missing capability
- **Resolution during implementation:** Add a dialect-rendered drop-and-recreate step to the relational schema layer, guarded so it fires only when the live `oauth_tokens` lacks a `host` column. **The guard is not optional:** without it every restart destroys valid tokens. Exercise it on all three engines through `crates/mcp-fs/src/storage/conformance.rs`.
- **Detected by:** E2E-NEW-037 fails on SQLite. If the guard is omitted, the existing `survives_reopen_on_disk` test (`crates/mcp-fs/src/git/oauth/persistence.rs:361`) fails.
- **Blocks which requirement:** FR-NEW-010, FR-NEW-011, and therefore FR-NEW-009 in persisted mode.
- **Status:** open

#### DRIFT-008: There is no identity middleware to place routes behind
- **Spec says:** FR-NEW-037, "authenticated by the existing RS256 identity layer"; §7.2, "The existing RS256 identity layer, not a loopback session".
- **Code does:** No identity middleware exists. `crates/mcp-fs/src/app.rs:5-7` states that identity verification "happens inline at the top of that handler, which is the only guarded route", and the REST plane repeats the resolution per handler (`crates/mcp-fs/src/api/dataplane.rs:177-178`).
- **Nature:** bypassed abstraction
- **Resolution during implementation:** Resolve identity inline at the top of each token screen handler using `state.identity.resolve`, exactly as `dataplane.rs:177-178` does, returning the 401 body shape documented at `app.rs:10-11`. Do not introduce a tower layer for this feature alone. FR-NEW-047's three-source resolution is implemented in that inline call.
- **Detected by:** E2E-NEW-172 and E2E-NEW-173 fail if a handler omits the inline check, since no layer would have performed it.
- **Blocks which requirement:** FR-NEW-037, FR-NEW-047, FR-NEW-048.
- **Status:** open

#### DRIFT-009: A blocking libgit2 call cannot be aborted at a deadline
- **Spec says:** FR-NEW-044, the server "SHALL abort it and fail with a distinct timeout error... releasing the per-repository write lock"; E2E-NEW-155 and E2E-NEW-156.
- **Code does:** libgit2 work runs as `spawn_blocking(... block_on(f()))` (`crates/mcp-fs/src/tools/git.rs:343-352`). A `tokio::time::timeout` around that future returns, but **does not stop the blocking thread**. `git2` 0.20.4 exposes no cancellation handle, only `RemoteCallbacks` progress callbacks, which do not fire during connect or the TLS handshake. The write lock is a `tokio::sync::Mutex` (`crates/mcp-fs/src/git/repo.rs:44`).
- **Nature:** missing capability
- **Resolution during implementation:** Acquire the write lock in the async caller rather than inside the blocking closure, so dropping the timed-out future releases it. Enforce the deadline twice: wrap the future in `tokio::time::timeout`, and return an abort from `RemoteCallbacks::transfer_progress` once the deadline has passed. Accept and document that an orphaned blocking thread can outlive the error until its socket times out.
- **Detected by:** E2E-NEW-156 fails, because the second operation blocks on the still-held lock.
- **Blocks which requirement:** FR-NEW-044.
- **Status:** open

#### DRIFT-010: `git.auth_status` never enumerates the token store
- **Spec says:** FR-MOD-003, "SHALL report one entry per stored `(person, host)`".
- **Code does:** `auth_status` iterates a fixed `PROVIDERS` constant and computes a boolean per provider (`crates/mcp-fs/src/tools/git_auth.rs:225-252`); it never enumerates the store. The only enumeration available is `OAuthTokenStore::list_ids` (`crates/mcp-fs/src/git/oauth/store.rs:174`), which returns pairs for **every person, unfiltered**.
- **Nature:** missing capability
- **Resolution during implementation:** Add a per-person enumeration to `OAuthTokenStore`, for example `list_for_person(person) -> Vec<(String, OAuthSession)>` filtering on the lowercased person part of the key, and build the response from it. **Using `list_ids` naively would leak every person's hosts and violate FR-NEW-040.**
- **Detected by:** E2E-MOD-002 fails, because a host with no token would still be listed. E2E-NEW-162 fails, because bob would see alice's hosts.
- **Blocks which requirement:** FR-MOD-003, FR-NEW-038, and the isolation guarantee of FR-NEW-040.
- **Status:** open
