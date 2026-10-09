# GitHub Enterprise Support and Git Token Store Seeding — Design Document

> Id: SPEC-0011 (design)
> Status: implemented
> Migrated from: specs/archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md on 2026-10-09

---

## 1. Components

| Component | File | Role |
|---|---|---|
| Host resolution | `crates/core/src/git/remote.rs` | Owns `validate_hosts`, `resolve_host`, URL validation, credential supply, timeout, and the four remote operations (clone/push/fetch/pull). The single implementation of the security pipeline (DEC-034/DEC-039). |
| Tool adapters | `crates/core/src/tools/git.rs` | Thin `#[tool]`-level adapters (`tool_remote_clone`, `tool_remote_push`, `tool_remote_fetch`, `tool_remote_pull`) calling into `git::remote`; never reimplements host resolution, credential supply or timeout. |
| Auth tool adapters | `crates/core/src/tools/git_auth.rs` | `git.auth`, `git.auth_status`, `git.auth_revoke`, `git.token_set`. |
| Token custody | `crates/core/src/git/oauth/store.rs` | `OAuthTokenStore`, keyed `(person, host)`; `has_valid_token`, `require_valid_credential`, `list_for_person`. |
| Token persistence | `crates/core/src/git/oauth/persistence.rs` | `oauth_tokens` table, primary key `(person, host)`, `host` as a bounded `TextKey`. Drop-and-recreate on missing `host` column. |
| Remote index | `crates/core/src/git/db.rs` | `git_remotes` table; `add_remote`/`list_remotes`, now with its first production writer (clone). |
| Merge engine | `crates/core/src/git/merge.rs` | Shared three-way merge, `MergeOptions::file_favor`, the response types every combine operation serializes through. |
| Config | `crates/core/src/config.rs` | `GitConfig.hosts` (`HostMap`), `GitConfig.remote_timeout_secs`; boot calls `git::remote::validate_hosts`, never inspects an entry itself. |
| Token screen | `crates/core/src/token_screen.rs` | `/app/tokens`, `/app/tokens/revoke`; session-cookie + CSRF; delegates to `tools::git_auth`/`store`. |
| Migration | `crates/core/src/migrate.rs` | Copies `oauth_tokens` under the new key between backends. |
| Tool contract | `TOOL_CONTRACT.txt`, `tool-contract-golden.json` | Four new tools, three modified schemas, regenerated. |

---

## 2. Flows

**Clone (SC-004/SC-005).** Authorize mount → parse URL → reject non-HTTPS or userinfo-bearing →
`resolve_host` (exact match, case-insensitive) → miss fails before any socket opens → hit routes
to `anonymous` (no credential) or to a token lookup keyed `(person, host)` with expiry check →
clone via `Cred::userpass_plaintext("oauth2", token)` → import objects/refs → `add_remote`
upserts `origin` → one audit entry + one `git.remote` tracing span, success or failure.

**Push (SC-009/SC-010).** Authorize → read `origin` via `list_remotes`, parse its host → resolve
provider/token/expiry → export volume to temp working dir → push named branch → remote decides
fast-forwardness; a non-fast-forward rejection surfaces as a distinct `ERR_INVALID_ARGUMENT`
(`push refused: not a fast-forward`) → on success, advance `refs/remotes/origin/{branch}`.

**Fetch (SC-011).** Same host/token resolution → single refspec
`+refs/heads/*:refs/remotes/origin/*`, no tag following → update `refs/remotes/origin/*` only →
a remote-tracking ref whose branch vanished upstream is kept and reported in `refs_stale`, never
pruned.

**Pull (SC-012/SC-013/SC-015).** `branch` must equal the checked-out branch (rejected before
fetch otherwise) → volume must be clean relative to the branch tip (rejected before fetch
otherwise) → fetch runs as above → ancestry test via `graph_descendant_of`:
  - ancestor → fast-forward: advance branch ref, update files, report `old_sha`/`new_sha`/
    `files_changed`.
  - not an ancestor, no `on_conflict` → refuse with `pull refused: not a fast-forward`, but keep
    the fetch's results (objects + remote-tracking refs).
  - not an ancestor, `on_conflict` is `ours`/`theirs` → three-way merge via `merge_commits` with
    `MergeOptions::file_favor`; merge commit authored by the Person, auto-generated message
    naming the source ref, target branch and strategy; no conflict markers ever written.
  - quota charged on the delta only (changed blobs), before any write, atomically (nothing
    applied and the ref unadvanced on any write failure or insufficient quota).

**Token screen (SC-006).** `GET /app/tokens` resolves identity (forwarded header → `Authorization`
→ read-only `mcpfs_token` cookie, verified exactly as a bearer token), issues a single-use
`csrf_token` bound to that person, renders rows ordered by host ascending, never a token value.
`POST /app/tokens` / `POST /app/tokens/revoke` require the matching `csrf_token` when the request
was authenticated by the ambient cookie (header-authenticated requests are exempt), and delegate
to the same `git.token_set`/`git.auth_revoke` logic, never a second implementation. The server
never emits `Set-Cookie` for `mcpfs_token`.

---

## 3. Interfaces

**New tools.** `git.token_set(host, token, expires_at?)`; `git.remote_push(mount_id, branch)`;
`git.remote_fetch(mount_id)`; `git.remote_pull(mount_id, branch, on_conflict?)`.

**Modified tools.** `git.auth(provider, instance_url?, host?)`; `git.auth_status(provider?,
host?)`; `git.auth_revoke(provider?, host?)` (at least one of the two required).

**Fixed response shapes.**
- `git.remote_fetch` → `{refs_updated: [{ref, old_sha, new_sha}], refs_stale: [ref], objects_fetched: int, up_to_date: bool, auth: string}`.
- `git.remote_push` → `{branch, created: bool, up_to_date: bool, remote_sha: <40 hex>, auth}`.
- `git.auth_status` → `{statuses: [{host, provider, validity: "valid"|"expired", expires_at, scopes}]}`, ordered by host ascending.
- `git.auth_revoke` → `{host, provider: string|null, revoked: bool}`.

**HTTP routes.** `GET /app/tokens`, `POST /app/tokens` (form fields `host`, `token`,
`csrf_token`), `POST /app/tokens/revoke` (form fields `host`, `csrf_token`), registered only when
`git.enabled`.

**Error contract (fixed codes/prefixes, no new `ERR_*` added).**
- non-fast-forward push → `ERR_INVALID_ARGUMENT`, `push refused: not a fast-forward`
- dirty-volume pull → `ERR_INVALID_ARGUMENT`, `pull refused: volume has uncommitted changes`
- diverged pull, no strategy → `ERR_INVALID_ARGUMENT`, `pull refused: not a fast-forward`
- remote timeout → `ERR_INTERNAL_ERROR`, `remote timeout`
- no stored token → `ERR_UNAUTHENTICATED`, `no token for host`
- expired stored token → `ERR_UNAUTHENTICATED`, `token expired for host`

---

## 4. Data and State

| Entity | Storage | Key | Notes |
|---|---|---|---|
| `HostEntry`/`HostMap` | In-memory, from YAML | `host` | Immutable after boot. |
| `Provider` | In-memory enum | — | `Github`\|`Gitlab`\|`Generic`\|`Anonymous`. |
| `OAuthSession` | `oauth_tokens` | — | `provider` is a non-key attribute; `access_token`/`scopes`/`expires_at`/`instance_url` unchanged shape. |
| `oauth_tokens` row | Relational (SQLite/PostgreSQL/SQL Server) | `(person, host)` | `host` is `TextKey`-bounded. Legacy `(person, provider)` rows dropped on upgrade via a guarded drop-and-recreate fired only when the live table lacks a `host` column. |
| `git_remotes` row | Relational | `(volume_id, name)` | Schema unchanged; clone is now its first production writer. |
| CSRF token | In-memory only (token screen) | person | UUIDv4, single-use, never persisted. |

---

## 5. Configuration

```yaml
git:
  hosts:
    github.com: github
    github.ibm.com: github
    gitlab.acme.corp: gitlab
    git.acme.internal: generic
    public.example.org: anonymous
  remote_timeout_secs: 120   # default

Boot validation (`ServerConfig::validate` → `git::remote::validate_hosts`) rejects: an unknown
provider value, a duplicate host, and a malformed host (scheme, path, port, or wildcard
character present). `remote_timeout_secs` bounds every clone/push/fetch/pull.

---

## 6. Observability

- One `git.remote` tracing span per remote operation (`operation`, `host`, `provider`, `branch`
  when applicable, `outcome` of `ok`/`error`, `duration_ms`), emitted even for a failure resolved
  before any network call. Never carries a token, an `Authorization` value, or URL userinfo.
- One `safety.record_audit` entry per remote operation, naming the tool, host, branch (when
  named) and outcome.
- Never traced/logged/returned: token values, bearer headers, URL userinfo.

---

## 7. Decisions

Renumbered sequentially from the legacy `DEC-001`..`DEC-038` (several ids were retired or
superseded in the original log; this table keeps every surviving decision under a new sequential
id while preserving the legacy id for backlog lookups, per the "Legacy mapping" table below).

| New id | Decision | Rationale |
|---|---|---|
| DEC-101 | Scope is the complete solution: host map, seeding tool, web token screen | Partial delivery leaves the consuming agent degraded |
| DEC-102 | Providers stay `github`/`gitlab` plus explicit `generic`; no hardcoded host values | Keeps the credential shape correct for enterprise GitHub; avoids an open string set |
| DEC-103 | Token identity is per host, not per provider | One person may hold distinct tokens per host under the same provider |
| DEC-104 | Key is `(person, host)`; provider is a row attribute | `(person, provider, host)` would permit two tokens per host with no known use |
| DEC-105 | Pre-existing `oauth_tokens` rows are dropped at upgrade, not migrated | Inferring a host from a legacy provider value writes a guess into a primary key |
| DEC-106 | The three auth tools gain an optional `host` | A provider-only revoke is ambiguous under a per-host key |
| DEC-107 | *(superseded by DEC-128)* Screen was to be a tool returning a self-expiring URL | — |
| DEC-108 | The screen never displays a token value | Lists hosts, seeds, revokes only |
| DEC-109 | Non-goals: refresh/rotation, providers beyond the four, SSH, runtime map editing, credential-shape change | The HTTPS-password shape already works for read and write |
| DEC-110 | An undeclared host is a configuration error, not a silent downgrade to anonymous | Explicit over implicit; the previous substring-match defect went unnoticed under implicit anonymity |
| DEC-111 | Scope grows to full bidirectional remote sync | A clone-only token cannot serve the write path |
| DEC-112 | Both `git.remote_fetch` and `git.remote_pull`, pull refusing a non-fast-forward unless a strategy is given | The volume is the working tree; unconstrained merging needs no in-volume conflict representation |
| DEC-113 | Push is fast-forward only; no force | Keeps the remote authoritative on fast-forwardness |
| DEC-114 | Push takes an explicit `branch`, creates it on the remote when absent | — |
| DEC-115 | Host matching is exact only; no wildcards | Wildcards reintroduce the precedence ambiguity substring matching caused |
| DEC-116 | Seeding tool named `git.token_set` | — |
| DEC-117 | `git.token_set`'s `expires_at` is optional; absent means non-expiring | A PAT may genuinely never expire |
| DEC-118 | An omitted `host` on `git.auth` defaults to the provider's canonical public host | Keeps every existing caller working unchanged |
| DEC-119 | An expired stored token is absent-with-a-reason; operations fail fast | — |
| DEC-120 | An expired token is left in the store, not deleted on detection | Lets status report `expired` rather than absent |
| DEC-121 | Push/fetch/pull take no `url`; they resolve the stored `origin`; no remote-management tools | `git_remotes` already existed, unused |
| DEC-122 | `git.remote_clone` must persist the clone URL as `origin` | Push/fetch/pull have no other source for the URL |
| DEC-123 | Pull is atomic; a failed file write leaves the ref unadvanced | A pull-advanced ref over stale files is a silent HEAD inconsistency |
| DEC-124 | `on_conflict` of `ours`/`theirs`, absent refuses a non-fast-forward pull, present merges | Closes the divergence dead end without interactive resolution |
| DEC-125 | A dirty volume still refuses the pull | Committing first no longer strands the volume, once DEC-124 exists |
| DEC-126 | The merge commit is authored by the authenticated person | — |
| DEC-127 | The merge commit message is auto-generated, recording the strategy; no override | Keeps the resolution visible in `git.log` permanently |
| DEC-128 | Token screen served from the main server behind the RS256 identity layer, superseding DEC-107 | The loopback precedent (`doc.open_editor`) is unauthenticated against other local processes and unreachable remotely |
| DEC-129 | `git.auth_revoke` never revokes across hosts | Different hosts hold different tokens |
| DEC-130 | `git.auth_revoke` with `host` omitted defaults to the canonical public host | Uniform with DEC-118 |
| DEC-131 | A single spec, partitioned into stories by `/plan-spec` | Push/fetch/pull are inert without the keying and mapping work |
| DEC-132 | `git.remote_timeout_secs`, configurable, default 120 | — |
| DEC-133 | Seeded tokens longer than 8192 characters are rejected | Denial-of-service bound, consistent with existing bounded `TextKey` columns |
| DEC-134 | Approach B: extract `git/remote.rs` owning host resolution/credential supply/the four remote operations; `tools/git.rs` becomes a thin adapter | One implementation means one set of security proofs, not four |
| DEC-135 | A pull charges the write quota only on bytes of files actually changed, not the whole target tree | Charging the whole tree would refuse ordinary pulls on large repositories |
| DEC-136 | Fetch does not prune remote-tracking refs for branches deleted upstream; reports them in `refs_stale` | Pruning is a destructive ref operation nobody asked for |
| DEC-137 | The token screen accepts a read-only `mcpfs_token` cookie in addition to the two bearer headers, cross-origin submissions refused by a single-use `csrf_token` | A browser cannot attach `Authorization` to a top-level navigation; a header-only screen is unreachable by the only client that can use it |
| DEC-138 | The test plan deviated from the independent-test-designer mandate; author-designs-own-tests bias is present (process deviation, not a design choice) | The sub-agent tool returned HTTP 403 on every permitted model across three attempts |

(The legacy log's `DEC-035` through `DEC-038` map to `DEC-138`, `DEC-135`, `DEC-136`, `DEC-137`
above respectively; see "Legacy mapping" for the exact old→new correspondence used by any
backlog entry searching by the old id.)

---

## 8. Requirement to Code Map

See Section 6 (Functional Requirements, Evidence column) of `spec.md` for the full FR→code
mapping; it is not duplicated here to avoid drift between the two documents.

---

## 9. Legacy mapping

Source: specs/archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md (pre-move, renumbered to SPEC-0011)

Representative commit touching this legacy spec's story index: `0edb37e` ("Migrate legacy specs
to SPEC-NNNN protocol directory structure").

### Decisions (old → new)

| Old id | New id | Note |
|---|---|---|
| DEC-001 | DEC-101 | Resolved, implemented |
| DEC-002 | DEC-102 | Resolved, implemented |
| DEC-003 | DEC-103 | Resolved, implemented |
| DEC-004 | DEC-104 | Resolved, implemented |
| DEC-005 | DEC-105 | Resolved, implemented |
| DEC-006 | DEC-106 | Resolved, implemented |
| DEC-007 | DEC-107 | Superseded by DEC-128 (old DEC-028); recorded, not implemented |
| DEC-008 | DEC-108 | Resolved, implemented |
| DEC-009 | DEC-109 | Scope boundary, not a numbered requirement |
| DEC-010 | DEC-110 | Resolved, implemented; cited by backlog items re: anonymous-clone behaviour break |
| DEC-011 | DEC-111 | Resolved, implemented |
| DEC-012 | DEC-112 | Resolved, implemented |
| DEC-013 | DEC-113 | Resolved, implemented; cited by backlog "force push" deferral |
| DEC-014 | DEC-114 | Resolved, implemented |
| DEC-015 | DEC-115 | Resolved, implemented |
| DEC-016 | DEC-116 | Resolved, implemented |
| DEC-017 | DEC-117 | Resolved, implemented |
| DEC-018 | DEC-118 | Resolved, implemented |
| DEC-019 | DEC-119 | Resolved, implemented |
| DEC-020 | DEC-120 | Resolved, implemented |
| DEC-021 | DEC-121 | Resolved, implemented; cited by backlog "remote management tools" deferral |
| DEC-022 | DEC-122 | Resolved, implemented |
| DEC-023 | DEC-123 | Resolved, implemented |
| DEC-024 | DEC-124 | Resolved, implemented; cited by backlog "PR creation/squash merge/interactive conflict resolution" deferrals |
| DEC-025 | DEC-125 | Resolved, implemented |
| DEC-026 | DEC-126 | Resolved, implemented |
| DEC-027 | DEC-127 | Resolved, implemented |
| DEC-028 | DEC-128 | Resolved, implemented |
| DEC-029 | DEC-129 | Resolved, implemented |
| DEC-030 | DEC-130 | Resolved, implemented |
| DEC-031 | DEC-131 | Process decision, not a numbered requirement |
| DEC-032 | DEC-132 | Resolved, implemented |
| DEC-033 | DEC-133 | Resolved, implemented |
| DEC-034 | DEC-134 | Resolved, implemented; cited by backlog "pluggable CredentialProvider trait" deferral |
| DEC-035 | DEC-138 | Process deviation, not implementable as code |
| DEC-036 | DEC-135 | Resolved, implemented |
| DEC-037 | DEC-136 | Resolved, implemented; cited by backlog "fetch --prune" divergence-review item |
| DEC-038 | DEC-137 | Resolved, implemented |

### Drift findings (old → resolution)

| Old id | Resolution | Note |
|---|---|---|
| DRIFT-003 | `url` crate promoted to a direct workspace dependency | Resolved, implemented — `crates/core/src/git/remote.rs` parses hostnames via `url::Url` |
| DRIFT-004 | `migrate()` extended to copy `oauth_tokens` via `git::oauth::persistence::schema()` | Resolved, implemented — `crates/core/src/migrate.rs` |
| DRIFT-005 | Relational schema layer gained a guarded drop-and-recreate step, firing only when the live table lacks the `host` column | Resolved, implemented — `crates/core/src/git/oauth/persistence.rs:31-60` (`recreate_if_missing_column`) |
| DRIFT-008 | Identity resolved inline per handler in `token_screen.rs`, matching the `dataplane.rs` precedent; no new tower layer introduced | Resolved, implemented — `crates/core/src/token_screen.rs` |
| DRIFT-009 | Write lock acquired in the async caller (not inside the blocking closure) so a timed-out future releases it; deadline enforced via `tokio::time::timeout` plus a transfer-progress abort | Resolved, implemented — `crates/core/src/git/repo.rs`, `crates/core/src/tools/git.rs` |
| DRIFT-010 | `OAuthTokenStore::list_for_person` added, filtering on the lowercased person key; `auth_status` built from it instead of the fixed `PROVIDERS` constant | Resolved, implemented — `crates/core/src/git/oauth/store.rs:301` |

### Stories

| Legacy id | Title | Note |
|---|---|---|
| US-001 | The git.hosts map, its boot validation, and its single owner | done, implemented, not re-tracked in v2 queue |
| US-002 | Resolve a remote URL to a provider by parsing and exact match | done, implemented, not re-tracked in v2 queue |
| US-003 | Token identity becomes (person, host), in memory, on disk, and across backends | done, implemented, not re-tracked in v2 queue |
| US-004 | git.token_set: seed a token the caller already holds | done, implemented, not re-tracked in v2 queue |
| US-005 | An expired token fails the operation before any socket opens | done, implemented, not re-tracked in v2 queue |
| US-006 | git.auth takes a host, git.auth_status reports one entry per held host | done, implemented, not re-tracked in v2 queue |
| US-007 | git.auth_revoke removes exactly one host and never reaches across hosts | done, implemented, not re-tracked in v2 queue |
| US-008 | Extract git/remote.rs, validate the URL, and record origin at clone | done, implemented, not re-tracked in v2 queue |
| US-009 | git.remote_push: send one branch, fast-forward only | done, implemented, not re-tracked in v2 queue |
| US-010 | git.remote_fetch: objects and remote-tracking refs, nothing else | done, implemented, not re-tracked in v2 queue |
| US-011 | git.remote_pull: fast-forward, applied atomically | done, implemented, not re-tracked in v2 queue |
| US-012 | A pull charges the write quota for the bytes it actually writes | done, implemented, not re-tracked in v2 queue |
| US-013 | A diverged pull: refuse, or merge under one global strategy | done, implemented, not re-tracked in v2 queue |
| US-014 | Remote timeout, lock release, and the frozen failure messages | done, implemented, not re-tracked in v2 queue |
| US-015 | The token screen: routes, and identity from three sources | done, implemented, not re-tracked in v2 queue |
| US-016 | The screen lists held hosts, ordered, and never another person's | done, implemented, not re-tracked in v2 queue |
| US-017 | The screen seeds and revokes, protected by a single-use csrf_token | done, implemented, not re-tracked in v2 queue |
| US-018 | Every remote operation is audited and traced, and no token is ever emitted | done, implemented, not re-tracked in v2 queue |
| US-019 | Regenerate the frozen tool contract at 63 tools | done, implemented, not re-tracked in v2 queue |
