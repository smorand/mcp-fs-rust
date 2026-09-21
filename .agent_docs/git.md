# Git subsystem

Opt in (`git.enabled`). When on, the server registers the 14 `git.*` tools plus
the 4 `git.auth*` tools (`git.auth`, `git.auth_status`, `git.auth_revoke`,
`git.token_set`) and mounts the git smart HTTP routes, so a volume can be
cloned, fetched and pushed with a plain `git` CLI, including against a
GitHub Enterprise Server host declared in `git.hosts`.

Modules: `git/db.rs`, `git/odb.rs`, `git/repo.rs`, `git/remote.rs`, `git/http/`,
`git/oauth/`, `token_screen.rs`, tools in `tools/git.rs` and `tools/git_auth.rs`.

## Object storage

A git object is stored in the **volume's blob backend** under the key
`git:{sha}`, holding the canonical git bytes:

```text
{type} {len}\0{payload}      type: blob | tree | commit | tag
```

That is exactly what libgit2 hashes to get the object id, so the id can be
recomputed from the stored bytes alone. `git:` prefixes the key because file
blobs are keyed by a bare hex sha256; the prefix makes a collision impossible
while keeping one bucket per volume. `git/odb.rs` owns
`serialize`/`deserialize`/`object_id` plus read, write, exists, prefix resolution
(short sha) and listing.

Scope note: the key contains a colon, which is illegal in an NTFS filename, so the
local blob backend cannot hold git objects on a Windows host. Windows is out of scope
for this port (POSIX hosts only), so the key is left readable as is; a Windows target
would need a key mapping in the local backend, and the S3 backend is unaffected.

## Relational index and on disk paths

| Path | Content |
|---|---|
| `state/git/{project_id}.db` | `git_objects(volume_id + hash PK, type, size)`, `git_refs(volume_id + name PK, target, symbolic)`, `git_remotes(volume_id + name PK, url)` |
| `state/git-repos/{project_id}/` | bare libgit2 directory (HEAD, config, hooks) plus a rebuildable object cache |
| blob bucket, key `git:{sha}` | the objects themselves, source of truth |
| `state/oauth.db` | encrypted OAuth tokens, only when `MCPFS_TOKEN_KEY` is set |

The index exists because the blob store cannot be enumerated cheaply: it answers
"which objects exist", "what type and size", and short sha prefix lookups. Refs
live there too, which is why `git.status` needs no libgit2 call.

The two `.db` paths above are the SQLite default. The index is configured by `infra.git`
and the token store by `infra.oauth`, so either can run on PostgreSQL or SQL Server
instead, in which case one database holds every project and `volume_id` separates them
(see [`backends.md`](backends.md)). **`state/git-repos/` always stays on disk**, whatever
the index backend, because libgit2 needs a real working directory.

## Repository store and the write lock

`GitRepoStore` (process singleton, shared by the tools and the HTTP routes) keeps
one `GitRepoEntry` per project:

* `repo`: the `git2::Repository` behind a mutex (`Send` but not `Sync`).
* `db`, `objects`, `blobs`: the index, the blob backed object db, the bucket.
* `write_lock`: a separate mutex held for the whole of a `git-receive-pack`, so
  two concurrent pushes cannot interleave ref updates.

`git.init` creates the entry and points HEAD at `refs/heads/main`; it is
idempotent. `get_or_open_repo` opens the on disk state on demand, so the server
keeps working after a restart with a cold in process map. `purge_repo` deletes the
index db and the bare directory (used by `admin.delete_project`), while
`teardown_repo` only drops the in process entry.

All libgit2 work runs on the blocking pool (`on_git_thread`), because pack
building and diffs are CPU bound and a future holding a `Repository` is not
`Send`.

### No custom libgit2 ODB backend

The reference registers a `LibGit2Sharp.OdbBackend` subclass so libgit2 reads
straight from the blob store. `git2` cannot express that from safe Rust
(`git2::Odb` exposes only disk alternates and mempack; a real `git_odb_backend`
means C function pointers plus `git_odb_backend_malloc` lifetimes through
`git2-sys`). Instead the blob store stays the source of truth and is synced around
libgit2 calls: `export_to_repo` hydrates the on disk ODB before libgit2 needs
objects (log, diff, blame, pack building), `import_from_repo` copies anything
libgit2 wrote (an indexed incoming pack, a commit created by a tool) back into the
blob store and the index. Stored bytes are identical; the only visible effect is
that `state/git-repos/{project}/objects/` becomes a rebuildable cache instead of
staying empty. Deleting it loses nothing.

## HTTP smart protocol

| Method | Route | Purpose |
|---|---|---|
| GET | `/git/{mount_id}/info/refs?service=git-upload-pack` | ref advertisement for clone and fetch |
| GET | `/git/{mount_id}/info/refs?service=git-receive-pack` | ref advertisement for push |
| POST | `/git/{mount_id}/git-upload-pack` | clone / fetch |
| POST | `/git/{mount_id}/git-receive-pack` | push |

Any other `service` value is 400. Responses carry the git content types and the
no cache header trio.

Auth gate (`git/http/mod.rs::gate`), in order:

1. Resolve the bearer. `Basic` auth is accepted with the **token as the
   password** (any username), which is how a `git` CLI authenticates: `git clone
   https://x:$TOKEN@host/git/{mount_id}/`. A 401 carries
   `WWW-Authenticate: Bearer realm="mcp-fs"`; note the CLI only prompts for
   credentials on a `Basic` challenge, so anonymous CLI use needs
   `git.anonymous_read` or an explicit credential helper.
2. No identity is allowed only for a **read** (`info/refs?service=git-upload-pack`
   and `git-upload-pack`) and only when `git.anonymous_read` is true. Push always
   needs an identity, by definition.
3. An identified caller must be a **project member**. There is no platform admin
   bypass: git traffic is project data.
4. The repository must be initialized, else 404.
5. On push, a body above `git.max_pack_size_mb` is 413, enforced both by the axum
   body limit layer and by an explicit check.

Capabilities are advertised verbatim:
`multi_ack multi_ack_detailed side-band-64k ofs-delta agent=mcp-fs/0.1.0` for
upload-pack, `report-status delete-refs side-band-64k quiet atomic ofs-delta
agent=mcp-fs/0.1.0` for receive-pack.

There is no have/want negotiation: `have` lines are parsed and ignored, a NAK is
sent, and the pack carries everything reachable from the wanted tips. It is built
from a revwalk over those tips fed to `insert_walk`, not `insert_recursive`, which
would omit ancestry and break any repository with more than one commit. Pack data
goes out on side-band channel 1 in 65519 byte chunks, then a bare channel byte and
a flush. On push, the report is wrapped in band 1 when the client negotiated
`side-band-64k` and sent raw otherwise; without that wrapping git aborts with
"bad band" after the refs have already been updated.

## `git.*` tool behaviour

Every tool authorizes membership first (`AppState::authorize`), then requires
`git.init` to have run (`ERR_NOT_FOUND` with "call git.init first" otherwise).
Parameters and return keys are in `.agent_docs/tools.md`. Notable points:

* `git.status`, `git.branches`, `git.tags` read the indexed refs directly; `git.log`,
  `git.show`, `git.diff`, `git.blame` hydrate the on disk ODB from the blob store
  first, then use libgit2.
* `git.commit` builds a tree from the **current volume content** (there is no
  index or working tree), writes the commit through the blob backed object db,
  moves `refs/heads/{branch}` and sets HEAD on the first commit. Author defaults
  to the caller identity.
* `git.checkout_file` writes the file from a commit back into the volume and
  records an audit entry.
* `git.remote_clone` fetches a remote over HTTPS, copies the files into the volume
  and imports the history; it uses the OAuth token stored by `git.auth` for the
  detected provider when one exists, and reports which through the `auth` key.
* Ref resolution reproduces a reference quirk on purpose: a name made only of hex
  characters is treated as a sha *before* `refs/heads/{name}` is tried, so a
  branch named `beef` resolves to the sha `beef`. Short shas are 8 characters.

## The `git.hosts` host map (GitHub Enterprise support)

`git.hosts` (`config.rs`, on `GitConfig`) is a hostname-to-provider map declared
in config: `github.com: github`, an enterprise host like `github.ibm.com:
github`, a self-hosted GitLab like `gitlab.acme.corp: gitlab`, a plain HTTPS
server as `git.acme.internal: generic`, or a host meant for unauthenticated
clone/fetch only as `public.example.org: anonymous`. The four accepted
provider values are exactly `github`, `gitlab`, `generic` and `anonymous`
(`Provider::ALL` in `git/remote.rs`); `Github` also covers GitHub Enterprise
Server, since the credential shape and `git.auth`'s validation are provider
specific, not host specific.

Validation runs once at boot, from `ServerConfig::validate` calling
`git::remote::validate_hosts(&self.git)`: every key must be a bare hostname (no
scheme, path, port or wildcard, checked with `url::Host::parse`), every value
must be one of the four provider strings, and no host may repeat (a custom
`HostMap` `Deserialize` keeps every YAML key instead of `serde_yaml`'s default
of silently folding a repeated key to its last value, so a duplicate survives
to be rejected). A partial map is never published: a config with one bad entry
fails boot entirely rather than silently dropping just that entry. The
validated map lands in a process-lifetime `OnceLock<RwLock<HashMap<..>>>`, and
`git::remote::resolve_host` is the only reader.

Resolution (`resolve_host`) is **exact match only**: no wildcard, no
substring, no prefix fallback, and a host absent from the map is `Err`, not
silently `Provider::Anonymous` (an undeclared host must stay undeclared, never
implicitly trusted). `tools/git.rs` is the sole caller for an actual clone
URL: it parses the URL, lowercases the extracted host, and resolves it here.
`credentialed_hosts()` returns every declared host whose provider is not
`anonymous`, host-ascending; it is the one function the `/app/tokens` host
selector reads, so host resolution stays in this one file.

## Token identity is per `(person, host)`

The OAuth/PAT token store (`git/oauth/store.rs`) keys a session
`"{person}:{host}"`, both lowercased, **not** `"{person}:{provider}"`. One
person can hold two distinct tokens that are both provider `github`, one for
`github.com` and one for an enterprise host such as `github.ibm.com`; a
provider-only key could never tell those two apart. `provider` is kept on the
stored session as a non-key attribute (for display and for `git.auth`'s own
validation), never as part of the lookup key.

## `git.token_set`

`git.token_set(host, token, expires_at=null)` seeds a personal access token the
caller already holds for a host declared in `git.hosts`, bypassing the
interactive OAuth device flow entirely: useful when the caller (human or
automation) already has a PAT and would rather hand it over directly than
drive `git.auth`'s device code exchange. The provider is resolved from
`git.hosts` (`git::remote::resolve_host`), never supplied by the caller, so a
seeded token cannot be mis-filed under the wrong provider. The token is never
echoed back in the response. Implemented in `tools/git_auth.rs::token_set`,
the same function both the tool handler and the `/app/tokens` browser screen's
`POST /app/tokens` call, so there is exactly one seeding implementation.

## The remote pipeline (`git/remote.rs`)

One module owns host resolution, URL validation, credential supply and
clone/push/fetch/pull, so the security properties below are proved once
rather than once per operation:

* `validate_remote_url` runs first, before any network call and before any
  audit entry: only `https` is accepted (`ssh://`, `git://`, `file://`, plain
  `http://` and the bare `git@host:path` scp shorthand are all rejected,
  naming the scheme), and a URL carrying userinfo (`user:pass@host`) is
  rejected naming the host only, never the URL, since the URL itself carries
  the leaked credential.
* `resolve_host` (host to `Provider`) and the OAuth token lookup together
  decide the credential; `clone_to_temp` is the sole `git2::RemoteCallbacks`
  construction in the tree, wrapping any resolved token as
  `git2::Cred::userpass_plaintext("oauth2", token)`.
* On a successful `git.remote_clone`, the clone URL is recorded as the remote
  named `origin` for that volume (`git::db::add_remote`, upsert by name: a
  re-clone from a different URL replaces the row, including for an empty
  remote, which still gets a repository and an `origin` row).
* `require_origin(store, volume_id)` is the guard push, fetch and pull
  (US-009 to US-011) call before ever reaching the network: none of those
  three tools takes a `url` parameter, `origin` is their only source for one,
  and a volume that was never initialized or never cloned into fails with
  `ERR_INVALID_ARGUMENT` naming the missing remote.
* `tools/git.rs`'s `remote_clone` is a thin two-step wrapper: resolve the
  credential (`resolve_clone_credential`, which validates and parses the URL
  exactly once), then delegate to `clone_and_import` for the actual clone,
  object import, ref updates and audit entry. `clone_and_import` is also the
  function tests call directly with a local `file://` origin to exercise
  import mechanics, since `file://` can no longer reach `clone_and_import`
  through the registered tool at all.

## `git.remote_push`, `git.remote_fetch`, `git.remote_pull`

All three take no `url`: they resolve the volume's stored `origin` through
`require_origin`, the same guard clone records into (`ERR_INVALID_ARGUMENT`
when there is none). All three share `credential_callbacks`, the sole
`git2::RemoteCallbacks` construction in the tree, and are wrapped by
`with_remote_deadline` for the timeout (below) and by `run_remote_operation`
for exactly one `git.remote` tracing span plus one audit entry per call.

* **`git.remote_push(mount_id, branch)`**: pushes `refs/heads/{branch}` to the
  same name on `origin`, creating it remotely when absent. **Fast-forward
  only, no force**: the remote is authoritative on fast-forwardness (no local
  pre-flight check), and a non-fast-forward rejection reported through
  libgit2's `push_update_reference` is classified into a dedicated
  `ERR_NO_CLOBBER` (`classify_push_rejection`), distinct from any other
  rejection text the remote sends (wrapped as `ERR_INTERNAL_ERROR`, naming the
  branch verbatim) and from `ERR_UNAUTHENTICATED` for a missing/expired
  credential (raised earlier, before any network call).
* **`git.remote_fetch(mount_id)`**: a safe primitive. It downloads objects and
  updates `refs/remotes/origin/*` only, through one explicit refspec
  (`FETCH_REFSPEC`, `refs/heads/*:refs/remotes/origin/*`), with tag following
  disabled and pruning forced off. It **never touches `refs/heads/*` or any
  volume file**: the bare repository it runs against has no working tree to
  begin with. A branch deleted upstream is left exactly where it was rather
  than pruned, and is reported as stale rather than silently vanishing.
  `git.remote_pull`'s first step is literally a call to the same
  `fetch_from_remote` function.
* **`git.remote_pull(mount_id, branch, on_conflict=null)`**: only ever targets
  the checked-out branch (`branch` must equal what `HEAD` points at, checked
  before any network call); refuses a dirty volume. It fetches first (the
  exact `fetch_from_remote` primitive above), then tests ancestry
  (`Repository::graph_descendant_of`). A **fast-forward** applies directly:
  the new tree is written atomically to the volume, `on_conflict` is ignored
  even if supplied. A **diverged** history is refused (naming `on_conflict` as
  the way out) unless `on_conflict` is exactly `"ours"` or `"theirs"`
  (case-sensitive, any other value is `ERR_INVALID_ARGUMENT` up front, before
  any network attempt), in which case a real three-way merge
  (`Repository::merge_commits`) resolves **every** conflicting file by that
  strategy through `MergeOptions::file_favor`, and a merge commit is created
  with the previous local tip as first parent and the fetched remote tip as
  second. Either outcome's **write-quota charge is the delta only**
  (`charge_pull_quota`, the single authority for it): only the bytes that
  actually change between the old and new tree are charged, not the whole
  tree, and the charge happens before any write, so an insufficient quota
  refuses the pull and writes nothing.
* Both `push` and `fetch`/`pull` hold the same per-repository `write_lock`
  scope: nothing else can commit to or write into that repository between a
  pull's dirty check and its apply.

## `git.remote_timeout_secs`

One deadline (default **120** seconds, `GitConfig::remote_timeout_secs`)
bounds a single clone, push, fetch or pull, enforced by
`git::remote::with_remote_deadline`, the one wrapper all four operations
share. **Known limitation**: when the deadline fires, `tokio::time::timeout`
drops the awaited future and the caller's `?` unwinds, which releases the
per-repository `write_lock` (since that is what actually happens on drop) —
but the underlying blocking OS thread the network call was running on is
**not killed**: `git2` 0.20.4 exposes no cancellation handle, so that thread
keeps running until its own socket or TLS operation eventually errors out at
the OS layer. This is accepted, not hidden: the deadline guarantees the
caller gets its lock back and an error promptly, not that every OS thread the
timed-out call spawned stops immediately.

## The `/app/tokens` browser screen

`token_screen.rs` registers `GET/POST /app/tokens` and `POST
/app/tokens/revoke` on the **main HTTP server** (merged into the same router
as `/mcp` and `/api/fs`, only when `git.enabled`), not on a spawned loopback
session: a person can seed or revoke their own per-host token from a browser
without an MCP client.

There is no identity middleware in this codebase; the screen resolves the
caller inline, exactly like `api::dataplane` does, trying three sources in
order: the configured forwarded header, then `Authorization`, then a
read-only `mcpfs_token` cookie verified through the exact same
`IdentityResolver::verify` a header bearer token goes through. The cookie is
never set, refreshed or cleared by any response from this module, and no
route outside this module accepts it.

Seeding and revocation delegate to the exact `tools/git_auth.rs::{token_set,
auth_revoke}` functions the MCP tools call, never a second implementation;
listing reuses `git_auth::auth_status` directly, so the screen and
`git.auth_status` always agree on order and content. No token value is ever
rendered.

CSRF: `GET /app/tokens` issues a fresh, single-use `csrf_token` bound to the
requesting person, held only in this router's own in-memory `CsrfStore`
(never persisted, never part of `AppState`). Both `POST` routes require a
matching, unconsumed token, **but only when the request was authenticated
through the ambient `mcpfs_token` cookie**: a request carrying a bearer header
instead has no ambient credential a third party page could forge, so it is
exempt. A rejected CSRF check returns `403`
`{"error":"ERR_FORBIDDEN","detail":"missing or invalid csrf_token"}`.

## OAuth (device flow) and token persistence

`git.auth` runs the RFC 8628 device authorization grant:

| Provider | Device code endpoint | Token endpoint | Scopes |
|---|---|---|---|
| github | `https://github.com/login/device/code` | `https://github.com/login/oauth/access_token` | `repo` |
| gitlab | `{instance}/oauth/authorize_device` | `{instance}/oauth/token` | `read_repository write_repository` |

The tool answers immediately with `user_code` and `verification_uri` and a
detached task polls the token endpoint at the interval the provider asked for; the
client waits by calling `git.auth_status`. The client id comes from config, the
client **secret is read from the environment at call time** (named by
`git.github_client_secret_env` / `git.gitlab_client_secret_env`), so it is never
held in a field, never serialized, never logged. `DeviceCode`, `TokenPoll` and
`OAuthSession` redact their secrets in `Debug`.

Tokens live in `OAuthTokenStore`, keyed `"{person}:{host}"` lowercased (see
[Token identity is per `(person, host)`](#token-identity-is-per-person-host)
above), and belong to a person rather than to a mount. Persistence is **opt in
through `MCPFS_TOKEN_KEY`** (a base64 key that must decode to exactly 32
bytes; generate with `openssl rand -base64 32`). With it set, `state/oauth.db`
holds one row per `(person, host)` with the token encrypted AES-256-GCM as
`nonce(12) || tag(16) || ciphertext`; sessions load at startup and every mutation
writes through. Only the token is encrypted, the metadata is queryable clear text.
Without the variable the store is memory only and authentication is lost on
restart. A malformed key fails loudly and stays failed (the error is cached), so a
typo never silently downgrades to memory only. A row that fails to decrypt (key
rotation, corruption) is skipped rather than blocking the boot.

## Config flags

```yaml
git:
  enabled: false          # opt in; off means no git tools and no git routes
  object_format: sha1     # sha256 is accepted and ignored (bundled libgit2 is sha1 only)
  anonymous_read: false   # allow unauthenticated clone and fetch
  max_pack_size_mb: 512   # enforced on push bodies (413)
  github_client_id: ""
  github_client_secret_env: MCPFS_GITHUB_CLIENT_SECRET
  gitlab_client_id: ""
  gitlab_client_secret_env: GITLAB_CLIENT_SECRET
  gitlab_instance_url: https://gitlab.com
  remote_timeout_secs: 120   # deadline for one clone/push/fetch/pull
  hosts:                     # exact hostname -> github|gitlab|generic|anonymous
    github.com: github
    github.ibm.com: github    # GitHub Enterprise Server: same provider, distinct host
    gitlab.acme.corp: gitlab
    git.acme.internal: generic
```

## Divergences from the reference

Each one is a case where copying the reference would copy a defect, and each is
documented at its call site. Headlines only:

* the git protocol is actually usable (`multi_ack_detailed` advertised, full
  ancestry in the pack, `unpack ok` sent, report framed on band 1);
* membership is enforced on the HTTP routes;
* `git.max_pack_size_mb` is enforced;
* pushed objects are really indexed and imported;
* no custom libgit2 ODB backend (see above);
* `admin.delete_project` purges the git state instead of leaking it.

The authoritative list, with the reasoning and the non git divergences, is in
[`.agent_docs/parity.md`](parity.md). Do not restate it elsewhere.
