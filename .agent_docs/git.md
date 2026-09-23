# Git subsystem

Opt in (`git.enabled`). When on, the server registers the 39 `git.*` tools (including `git.merge`, whose engine is
`git/merge.rs`) plus
the 4 `git.auth*` tools (`git.auth`, `git.auth_status`, `git.auth_revoke`,
`git.token_set`) and the 6 `git.pr_*` tools (`git.pr_create`, `git.pr_list`,
`git.pr_get`, `git.pr_diff`, `git.pr_merge`, `git.pr_review`), 49 in all, and mounts the git smart HTTP routes, so a volume can be
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
* `require_remote(store, volume_id, remote)` is the guard push, fetch and pull
  call before ever reaching the network: none of those three tools takes a
  `url` parameter, a declared remote is their only source for one, and the
  lookup is the one `list_remotes` query that settles it. `origin` is the
  default; a volume that was never initialized or never cloned into fails with
  `ERR_INVALID_ARGUMENT` naming the missing `origin`, while any other name the
  caller supplied and that is not declared fails `ERR_NOT_FOUND` naming the
  remote and the volume (US-021, FR-NEW-146), always before a connection is
  opened.
* `tools/git.rs`'s `remote_clone` is a thin two-step wrapper: resolve the
  credential (`resolve_clone_credential`, which validates and parses the URL
  exactly once), then delegate to `clone_and_import` for the actual clone,
  object import, ref updates and audit entry. `clone_and_import` is also the
  function tests call directly with a local `file://` origin to exercise
  import mechanics, since `file://` can no longer reach `clone_and_import`
  through the registered tool at all.

## The shared merge engine and the conflict contract (`git/merge.rs`)

One implementation of "combine two histories, and report what could not be
combined", shared by `git.merge` and `git.remote_pull` and meant for rebase,
cherry_pick, revert and stash apply as they land. It owns:

* `three_way_merge(repo, ours, theirs, favor)` -> `Clean { tree }` or
  `Conflicted { conflicts }`. `favor` absent surfaces the conflicts instead of
  pre-resolving them (DEC-902), which is what `git.merge` and `git.remote_pull`
  both use; `favor` set is available but no caller sets it. Nothing is written on the
  conflicting path: the index is in memory and the merged tree is written only
  when the merge is clean.
* `diff_tree_changes`, `charge_quota`, `apply_changes_atomically`: the delta,
  the write-quota charge and the two pass all-or-nothing volume apply (pass 1
  pre-checks every write target, pass 2 deletes then writes). The quota is
  charged before the first byte moves, and the ref is advanced only after the
  apply returns.
* The response TYPES. `ConflictResponse`, `MergeResponse` and
  `OperationSummary` are `Serialize` structs; no call site hand builds a
  response object. The conflict response is exactly `status`, `operation`,
  `operation_id`, `source_ref`, `current_step`, `total_steps`, `conflicts`,
  `continue_with`, `abort_with`, each conflict exactly `path`, `ours`,
  `theirs`, `base`, `binary`, `type_change`, and each side exactly `exists`
  and `content`. `content` is null when the side deleted the path and when the
  path is binary, so bytes are never mangled into text. `current_step` and
  `total_steps` are top level, zero based, and null for a single step
  operation.

`continue_with` and `abort_with` come from `GitOpType` itself, so the caller
never infers which tool finishes a conflicted operation: merge, remote_pull,
stash_apply and stash_pop pause to `git.merge_resolve`/`git.merge_abort`,
rebase to `git.rebase_continue`/`git.rebase_abort`, and cherry_pick and revert
to their own pair.

`git.merge_resolve` completes both families, and branches on the row's
`op_type` once every conflict is settled (DRIFT 2026-09-22). A merge or a pull
commits the resolved tree with two parents and moves `refs/heads/{branch}`, and
reports `status merged` with the commit sha. A `stash_apply` or `stash_pop` is a
working tree operation whose CONFLICT happens to be a merge conflict, which is
the only reason it routes here: it writes the resolved files to the volume and
stops, so it reports `status applied` with a null `merge_commit`, creates no
commit and leaves the branch tip byte identical. A resolved pop drops its stash
entry at that point, the step the conflicted pop deferred so no work could be
lost; a resolved apply keeps it, exactly like the non conflicting pair.

A conflict is an `Ok` response, not an error, and it applies nothing: no file,
no commit, no ref move. The pause is a `git_operations` row (state
`conflicted`, never `paused`), so it survives a restart and `git.status`
reports it as its `operation` object. A conflict marker never enters the
volume: the volume is a filesystem other tools read.

`git.merge` validates in a fixed order, and the order is part of the contract:
uninitialized volume, no checked-out branch, merging a branch into itself,
unknown `source_ref` (`ERR_NOT_FOUND`), then the write lock, then the
in-progress guard, then the dirty volume refusal (`ERR_INVALID_ARGUMENT`
naming `git.commit` and `git.stash_save`), and only then any merge work. So an
unknown ref or a dirty volume never leaves a row, an object or an audit entry
behind. Ancestry decides the rest: an already merged source is
`already_up_to_date` with a null `merge_commit`, a source the branch is an
ancestor of fast-forwards and creates no object at all, anything else is the
three way merge.

## `git.remote_push`, `git.remote_fetch`, `git.remote_pull`

All three take no `url`: they resolve a declared remote through
`require_remote`, `origin` unless the optional `remote` parameter names another
one (US-021, default rendered in the JSON schema). All three share `credential_callbacks`, the sole
`git2::RemoteCallbacks` construction in the tree, and are wrapped by
`with_remote_deadline` for the timeout (below) and by `run_remote_operation`
for exactly one `git.remote` tracing span plus one audit entry per call.

* **`git.remote_push(mount_id, branch, remote="origin", remote_branch=null)`**:
  pushes `refs/heads/{branch}` to `refs/heads/{remote_branch}` on `remote`,
  defaulting the remote side to the local branch's name, creating it remotely
  when absent. The remote side name is validated by `validate_branch_name`, the
  same rule a local branch obeys, in the handler, before any remote is resolved
  and outside the audited operation. The remote-tracking ref written on success
  follows the REMOTE side's name: `refs/remotes/{remote}/{remote_branch}`, and
  the response carries `remote_branch` only when the caller supplied one. **Fast-forward
  only, no force**: the remote is authoritative on fast-forwardness (no local
  pre-flight check), and a non-fast-forward rejection reported through
  libgit2's `push_update_reference` is classified into a dedicated
  `ERR_NO_CLOBBER` (`classify_push_rejection`), distinct from any other
  rejection text the remote sends (wrapped as `ERR_INTERNAL_ERROR`, naming the
  branch verbatim) and from `ERR_UNAUTHENTICATED` for a missing/expired
  credential (raised earlier, before any network call).
* **`git.remote_fetch(mount_id, remote="origin")`**: a safe primitive. It
  downloads objects and updates `refs/remotes/{remote}/*` only, through one
  explicit refspec (`fetch_refspec(remote)`,
  `+refs/heads/*:refs/remotes/{remote}/*`), with tag following
  disabled and pruning forced off. It **never touches `refs/heads/*` or any
  volume file**: the bare repository it runs against has no working tree to
  begin with. A branch deleted upstream is left exactly where it was rather
  than pruned, and is reported as stale rather than silently vanishing.
  `git.remote_pull`'s first step is literally a call to the same
  `fetch_from_remote` function.
* **`git.remote_pull(mount_id, branch, remote="origin")`**: only ever targets the checked-out
  branch (`branch` must equal what `HEAD` points at, checked before any network
  call); refuses a dirty volume. It fetches first (the exact
  `fetch_from_remote` primitive above), then tests ancestry
  (`Repository::graph_descendant_of`). A **fast-forward** applies directly: the
  new tree is written atomically to the volume. A **diverged** history is
  merged by the shared engine with no caller decision (US-007, FR-MOD-104): a
  clean three-way merge creates a merge commit with the previous local tip as
  first parent and the fetched remote tip as second, message `Merge
  remote-tracking branch 'origin/{branch}' into {branch}`; a conflicting one
  applies nothing, records a `merge` row in `git_operations` and returns the
  shared conflict response, finished with `git.merge_resolve` or abandoned with
  `git.merge_abort`. The `on_conflict` parameter was REMOVED (FR-DEL-101,
  DEC-908): sending it is `ERR_INVALID_ARGUMENT` naming `git.merge_resolve`.
  Whatever the outcome, the fetch's results (the fetched objects and
  `refs/remotes/origin/*`) are retained, so a resolve or a retry costs no
  second download. Either applying outcome's **write-quota charge is the delta
  only**: only the bytes that actually change between the old and new tree are
  charged, not the whole tree, and the charge happens before any write, so an
  insufficient quota refuses the pull and writes nothing.
* Both `push` and `fetch`/`pull` hold the same per-repository `write_lock`
  scope: nothing else can commit to or write into that repository between a
  pull's dirty check and its apply.

## Stash (`refs/stash/*`)

A stash entry is a **commit**, not a row in a table of its own (DEC-913): the
snapshot tree is the volume's state at save time, the single parent is the HEAD
it was taken against (`base_sha`), and the entry is named by
`refs/stash/{commit sha}` in the existing `git_refs` table. Nothing else was
needed, and the namespace is reachable through no branch or tag tool, because
`git.branches` filters `refs/heads/` and `git.tags` filters `refs/tags/`.
`git.status` does list them, as it lists every non symbolic ref.

`branch` and a millisecond `created_at` have nowhere to live in `git_refs`
(name plus target) and nowhere in a git signature (seconds only), so
`git.stash_save` appends them to the commit message as one trailer line,
`\n\nmcp-fs-stash: {"branch":...,"created_at":...}`, and the tools strip it
back off at the LAST occurrence, so a caller's own message round-trips byte for
byte even if it contains the same text.

`git.stash_save` runs entirely under the per project write lock: it counts the
pool against `git.max_stash_entries` (default 100, refused rather than evicted,
because evicting destroys work), refuses a repository with no commits and a
volume with no changes against HEAD, writes the commit and the ref, and only
then reverts the volume through the shared `merge::charge_and_apply`, which
charges the quota before a byte moves. If that revert refuses, the ref is taken
back, so the pair is all or nothing. The ref is written first on purpose: a
failed revert then leaves the work saved rather than lost.

## git.rebase, the pre-flight half (US-013)

`git.rebase` takes `onto` plus an ordered `todo` of `{action, sha, message?}`,
where the action set is exactly `pick`, `squash`, `drop`, `reword` (DEC-918:
`edit`, `fixup`, `exec` and `break` are excluded, `exec` because it would run
arbitrary commands on the server). Every check happens before any commit is
created, any ref moves or any `git_operations` row is written, in this order:

1. pure, on the argument alone (`parse_todo`): array shape, non empty, at most
   `git.max_rebase_todo` entries, known action spelled exactly, no leading
   `squash`, no duplicate sha, no blank `reword` message.
2. relational, from `git_objects` and `git_refs`: the checked-out branch, its
   tip, `onto` resolved and known to be a commit, `onto` not already the tip,
   and every todo sha a known commit.
3. under the per project write lock, on the git thread: the dirty volume
   refusal (same wording as `git.branch_switch`), then the `up_to_date` answer
   when the branch already contains `onto`, then one revwalk of `onto..tip`
   stopped at the bound, then the range membership check in both directions
   (nothing outside the range, nothing of the range left out).

The walk is what the pre-flight costs, and it is bounded: it stops at
`git.max_rebase_todo + 1` commits, so a rejected todo never walks a long
history.

## git.rebase pause, continue and abort (US-015)

`replay_steps` walks the todo from an index, one `cherrypick_commit` per non
dropped entry. A clean run ends in `ReplayOutcome::Done` and `finish_rebase`
applies ONE tree to tree delta from the pre-rebase tip to the replayed tip,
charged before applied, then moves the branch on both sides of the dual write.

A conflicting entry ends in `ReplayOutcome::Paused` and `pause_replay` records
it (FR-NEW-219):

* the replayed commits are imported into the blob object store and named by
  `refs/mcp-fs/rebase-head`. That ref is the whole reason a resume replays only
  the REMAINING entries and survives a restart: the row carries no replay tip
  column, because `onto_sha` and `original_tip_sha` are already spoken for.
* the `git_operations` row holds `op_type=rebase`, `state=conflicted` (never
  `paused`), `source_ref` NULL (a rebase combines a plan, not a named ref),
  `onto_sha`, `original_tip_sha`, the full `todo`, the zero based
  `current_step`, `total_steps`, the REMAINING conflict paths and the
  resolutions recorded so far (an empty object, never NULL).
* nothing at all is written to the volume and the branch does not move, so
  `git.rebase_abort` is exact by construction: it writes the branch back to
  `original_tip_sha`, drops the pause ref and the row, and touches no file.

`git.rebase_continue` resolves the paused entry through `merge::resolve_index`
on the very index the pause reported (the cherry-pick is replayed, not cached,
for the same restart reason `git.merge_resolve` replays its merge), commits it,
and continues. A partial resolution is RECORDED and the call REFUSED
(FR-NEW-222), so the step index never moves backwards and a caller can resolve
over several calls. `git.rebase_continue` refuses anything that is not a rebase
and `git.merge_resolve` refuses a rebase, each naming the operation actually in
progress and its pair, read off `GitOpType` (FR-NEW-225).

## git.cherry_pick, continue and abort (US-016)

A cherry-pick is a ONE ENTRY replay, so it is literally `replay_steps` over a
single `pick` step: `pause_replay` records a pause, `land_replay` lands a clean
run and `replay_continue`/`replay_abort` finish or abandon it. Nothing about the
operation is implemented twice; what US-016 added to the shared machinery is the
`GitOpType` parameter on the pause, the `mainline` field on a todo step, and
`land_replay` split out of `finish_rebase` so a caller can build its own
response.

* the new commit keeps the ORIGINAL author and records the caller as committer,
  which is exactly what `RebaseAction::Pick` already did for a rebase.
* `status` is `committed`, `conflict` or `already_present`, never `ok`, and the
  response carries exactly `status`, `new_sha`, `source_sha`. `already_present`
  covers both ways a change can already be there: the commit is an ancestor of
  the tip (by sha), and replaying it produces the tree the branch already has
  (by content, which is the empty diff case). It is REPORTED with `new_sha`
  null, never an error and never a duplicate commit.
* `current_step`/`total_steps` are NULL on the wire and in `git.status`: a pick
  is single stepped. The columns are not nullable, so the row itself carries
  0 and 1; the response type is what makes the distinction.
* a merge commit without `mainline` is `ERR_INVALID_ARGUMENT` naming the parent
  count and the parameter; `mainline` is the 1-based parent index libgit2's
  `cherrypick_commit` takes.
* an unknown sha is `ERR_NOT_FOUND`, and so is a sha that exists but names a
  blob or a tree ("not a commit"). A value that is not hexadecimal at all is
  `ERR_INVALID_ARGUMENT` on `commit_sha`, settled before any repository opens.
* cost of one pick: one `git_objects` lookup, one hydrate, one clean volume
  check, one libgit2 `cherrypick_commit`, one tree to tree diff, and a write of
  only the paths that diff names.

Two test level decisions, deliberately different from the story's test bodies:
the volume is written only at completion (the story asserts both that and the
opposite on the same fixture, and "nothing applied at a pause" is the shared
conflict model), and a `theirs` resolution takes the replayed side's blob
whole, so the entries after it replay cleanly; a multi pause scenario therefore
resolves with `ours`.

`git.stash_drop` removes the ref only. The commit object stays in the store,
exactly as `git.branch_delete` leaves its commits, so a mistaken drop is still
recoverable by whoever kept the id. `git.stash_list` takes no lock at all.

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
| github | `https://github.com/login/device/code` | `https://github.com/login/oauth/access_token` | `repo` (`git.github_scope`) |
| gitlab | `{instance}/oauth/authorize_device` | `{instance}/oauth/token` | `api read_repository write_repository` (`git.gitlab_scope`) |

Both scope sets are configuration, not constants (FR-NEW-331); the table holds
the defaults. GitLab's includes `api` because GitLab documents
`write_repository` as Git over HTTP access only, granting **no** REST API
access, so a token without `api` cannot reach the merge request surface
(FR-MOD-109). A GitLab token granted before this change must be re-granted with
`git.auth`, or replaced with `git.token_set`.

### The pull request scope gate

`crates/mcp-fs/src/git/oauth/scopes.rs` judges the scope set recorded with a
token: `Capable`, `Insufficient { missing }`, or `Unknown`.
`OAuthTokenStore::require_pr_credential` is the single gate a `git.pr_*` tool
calls: presence and expiry first (so an expired *and* narrow token reports
expiry), then the scope check, both reading the one session that single lookup
returned, so the check costs no query and no round trip. A known-insufficient
set is `ERR_FORBIDDEN` before any request, naming the missing scope, the host,
and both `git.auth` and `git.token_set`. An **empty or unknown** set is
attempted instead and the provider's own refusal is surfaced (DEC-911):
`git.token_set` seeds PATs whose scopes the server cannot enumerate, and
refusing them up front would make that tool useless. `git.auth_status` reports
the same judgement per entry as `pr_read`, `pr_write`, `pr_capable` (null when
unknown) and `missing_scopes`.

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
  max_stash_entries: 100     # refs/stash/* entries one volume may hold
  max_rebase_todo: 200       # entries one git.rebase todo may hold
  github_scope: repo                                   # requested by git.auth
  gitlab_scope: api read_repository write_repository   # api is required for the REST API
  provider_api_timeout_secs: 30   # deadline for one provider REST round trip (git.pr_*)
  max_pr_diff_mb: 12              # cap on a git.pr_diff answer; past it, truncated: true
  hosts:                     # exact hostname -> github|gitlab|generic|anonymous
    github.com: github
    github.ibm.com: github    # GitHub Enterprise Server: same provider, distinct host
    gitlab.acme.corp: gitlab
    git.acme.internal: generic
```

## The provider client seam (`git/provider/`, US-024)

Every pull request tool goes through one seam, and it holds three things:

* `resolve_target(remote_url, instance_url)` turns a volume's remote into a
  `ProviderTarget { host, provider, base_url, owner, repo }`. It reuses
  `validate_remote_url` (scp shorthand, non https scheme, embedded credential)
  and the published `git.hosts` map, so GitHub Enterprise Server and self hosted
  GitLab need no new configuration: github.com goes to `https://api.github.com`,
  any other `github` host to `https://{host}/api/v3`, a `gitlab` host to
  `/api/v4` on the stored `instance_url` when there is one and on the remote's
  own host otherwise. `generic` and `anonymous` are `ERR_NOT_SUPPORTED`; a URL
  with no `/owner/repository` path is `ERR_INVALID_ARGUMENT`. All of this runs
  before any token is read and before any network call, which is what makes the
  refusal ordering assertable.
* `ProviderClient`, a trait, so a test injects a fake returning canned provider
  JSON and never reaches the network. Same seam shape as `DeviceFlowClient`.
* `HttpProviderClient`, the only real transport and the only place the
  `Authorization` header is set. It owns the safety rules: reqwest redirects are
  disabled and handled by hand so a redirect to a host other than the resolved
  API host fails the call rather than replaying the credential; a response body
  is read in chunks and refused past 16 MiB rather than buffered; the timeout is
  `git.provider_api_timeout_secs`. The credential's value is scrubbed out of
  every error message AND every response body, because a provider's own 401 body
  may echo the token back. `Credential`'s `Debug` prints `<redacted>`, and the
  request span carries only method, host and path.

One process wide client (`shared_client`), because it owns the connection pool.

## The pull request surface (`git.pr_*`, US-025)

`tools/git_pr.rs` is the whole family, six tools: `git.pr_create`,
`git.pr_list`, `git.pr_get`, `git.pr_diff`, `git.pr_merge` and `git.pr_review`.
`PrCall::open` IS the gate order, and no
tool may reorder it: `state.authorize(mount_id, person)`, then
`require_remote` for the declared remote, then `resolve_target` (offline), then
`OAuthTokenStore::require_pr_credential`, the single call that does both the
token lookup and the scope validation, and only then the transport. So an
insufficient scope set is refused before every network call, the head branch
pre-flight included. The tool name is prefixed onto the store's message,
because that message is shared by the whole family.

`git.pr_create` reads the head branch on the remote first (GitHub
`/repos/{owner}/{repo}/branches/{head}`, GitLab
`/projects/{owner%2Frepo}/repository/branches/{head}`) and a 404 there is
`ERR_NOT_FOUND` naming the branch, the host and `git.remote_push` as the
remedy, with no create issued. GitHub then takes
`{base, head, title, body, draft}`; GitLab takes
`{source_branch, target_branch, title, description}` and, since it has no draft
flag, carries draftness as a `Draft: ` title prefix, which the normalizer
strips back off so both providers report the same `title`. A provider side
refusal (duplicate pull request, protected branch, missing permission) keeps the
provider's status and message and is mapped onto the nearest `ERR_*`; it is
never reported as a success.

`git.pr_merge` (US-028) merges ON THE PROVIDER, and nothing local moves: the
answer carries a `note` naming `git.remote_fetch` as the way to observe it,
because silently fetching would run a network operation the caller did not ask
for. `strategy` is checked against `merge`/`squash`/`rebase` before anything is
resolved, then mapped: GitHub takes `merge_method` and rebases inside the same
call, GitLab takes a `squash` flag and needs its own `POST .../rebase` polled
to completion (`GET ...?include_rebase_in_progress=true`) before the merge. The
pull request is READ first on both providers, because that read is the only way
to tell "already merged" and "closed" apart from every other refusal: both
providers answer all of them with the same opaque 405, and a caller told "not
mergeable" about work that already landed would go and redo it. Every other
refusal (failing required check, missing review, protected branch, a strategy
disabled for that repository) is the provider's own status and message, mapped
onto the nearest `ERR_*`, with the refused strategy named; 405 maps to
`ERR_INVALID_ARGUMENT` on this surface. No refusal is ever retried with another
strategy.

`git.pr_review` (US-029) submits a verdict: `approve`, `request_changes` or
`comment`. Both the verdict and the body rule are pure argument checks, so an
unknown verdict, and a `request_changes`/`comment` with an empty or whitespace
only body, cost no lookup and no call. `approve` needs no body. The two
providers are genuinely different endpoints here, not a spelling difference:
GitHub posts ONE review to `/repos/{owner}/{repo}/pulls/{n}/reviews` with
`{"event": APPROVE|REQUEST_CHANGES|COMMENT}` plus `body` when supplied, and
reads the resulting state back out of its own answer. GitLab has no review
verdict at all, so `approve` is `POST .../approve` followed by a read of
`.../approvals` (the approve answer carries no approval information, and a
second approver may still be required), while `request_changes` is
`POST .../unapprove` then `POST .../notes`, in that order, so the merge request
is never left approved with a blocking note on it; a 404 on the unapprove means
there was nothing to withdraw and is not a failure. A `comment` is one note and
nothing else. `changes_requested` on GitLab IS the normalization: the provider
records no such state. The answer is six keys, `{pr_number, provider, host,
verdict, review_state, raw}`, not the normalized pull request: a review payload
names no pull request, and filling twenty one keys from it would put zeros where
the provider said nothing.

A refusal on a numbered pull request names the provider's own noun, the number
and the repository (`read pull request 9999 of 'acme/api'`), which is what makes
`ERR_NOT_FOUND` actionable. Two rules the whole family inherits from this story:
the quoted provider detail now keeps GitHub's `errors` array as well as its
`message` (GitHub puts "Can not approve your own pull request" there while
`message` says only "Unprocessable Entity"), and a provider 401 appends the
remedy, `re-authenticate with git.auth or git.token_set for host '<host>'`,
because a caller not told it will simply retry the rejected credential.

One normalized model, `git/provider/model.rs`, mapped into by
`PullRequest::from_github` and `from_gitlab`. Defining it once as a type is the
point: the audit of this surface caught wire shape drift four times when each
call site built its own JSON. Mapping rules worth knowing:

* `state` is derived, not copied. GitHub: `merged_at` non null or `merged` true
  means `merged`, otherwise `closed`/`open`. Without that, a closed and a merged
  pull request are indistinguishable. GitLab: `merged`, `closed`/`locked`,
  `opened`.
* `mergeable` is tri state. GitHub's `mergeable` may be present and null while
  it computes; GitLab's `merge_status` `checking`/`unchecked` is the same state.
  Neither collapses to `false`.
* counts are read tolerantly, because GitLab's `changes_count` is a STRING
  (`"4"`, and `"1000+"` past its cap), and a list payload omits counts entirely.
* timestamps are normalized to one spelling, so GitHub's `...Z` and GitLab's
  `....000Z` both come out as `+00:00`.
* `review_state` and `checks_state` come from OTHER endpoints, so a call that
  did not consult them reports `none`/`unknown` rather than inventing an answer
  (`PrSignals::unknown`).
* `raw` is the provider payload, untouched. It is the body the transport already
  scrubbed of the credential, so no request authorization data rides along.

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
