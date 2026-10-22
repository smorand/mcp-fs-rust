# Architecture

Three client facing surfaces, one engine, one storage layer. Nothing above
`core::fs_ops` holds filesystem logic, which is why the MCP tools and the REST
plane can never disagree.

```text
   MCP client              REST client / browser            git CLI
       |                          |                            |
 POST {mcp_path}          /api/fs/**   /api/docs        /git/{mount_id}/**
       |                          |                            |
+------v--------------------------v----------------------------v-----------+
| axum router (app::build)                                                 |
|   bearer JWT -> person; public: /health, /api/swagger.json, /api/docs     |
+------+--------------------------+----------------------------+-----------+
       |                          |                            |
+------v---------+     +----------v-----------+     +----------v---------+
| mcp::server    |     | api::dataplane       |     | git::http          |
| 94 tools       |     | 38 routes            |     | smart protocol v0  |
+------+---------+     +----------+-----------+     +----------+---------+
       |                          |                            |
       +------------+-------------+                            |
                    |                                          |
           +--------v---------+                     +----------v---------+
           | core::fs_ops     |                     | git::repo          |
           | core::diff docs::*|                    | git::odb, git::db  |
           +--------+---------+                     +----------+---------+
                    |                                          |
        +-----------v------------------------------------------v----------+
        | storage: VolumeClient (MetaBackend + BlobBackend), AdminBackend  |
        | RelationalMetaStore | LocalBlobStore / S3BlobStore | Relational- |
        | AdminStore                                          | ...        |
        +-----------+------------------------------------------+----------+
                    |
        +-----------v------------------------------------------------------+
        | storage::rel: RelationalDb / RelationalTx + Dialect              |
        | SqliteRelationalDb | PgRelationalDb | MssqlRelationalDb          |
        +------------------------------------------------------------------+
```

## Storage model

A volume is a pair: a metadata tree in a relational database and content addressed
bytes in a blob store. The two are bound by `storage::volume::VolumeClient`.

| Table | Where | Columns |
|---|---|---|
| `nodes` | `state/volumes/{project}.db`, or one shared database | `volume_id` + `path` (PK), `parent`, `name`, `kind` (dir/file), `size`, `mode`, `mtime`, `ctime`, `atime`, `sha256` |
| `blob_refs` | same db | `volume_id` + `sha256` (PK), `refcount`, `size` |
| `project`, `project_member` | `state/admin.db`, or one shared database | ACL registry, all identities normalized caseless |

Rules that the on disk contract depends on (`storage/meta.rs`,
`storage/volume.rs`):

* **Content addressing.** A file node stores the sha256 of its bytes; the bytes
  live once per distinct sha in the blob store. Two identical files share one
  blob.
* **An empty file stores no blob.** `sha256` is NULL, `size` is 0, and a read
  short circuits to an empty vector. Nothing is written to the blob store.
* **Copy is metadata only.** `copy_file` inserts a node pointing at the same sha
  and increments the refcount. No byte is moved, whatever the file size.
* **GC at refcount 0.** `put_file`, `delete_file` and `remove_subtree` decrement
  the refcount inside the same transaction and return the sha that reached 0;
  `VolumeClient` then deletes exactly that blob. A blob is never deleted while
  another node references it.
* **Overwrite preserves `ctime`.** `put_file` reads the previous row and reuses
  its `ctime`, so only `mtime` and `atime` move.
* **Local blob layout: `{infra.blob.dir}/{bucket}/{sha[..2]}/{sha}`** where
  `bucket` is `{bucket_prefix}{project_id}`. The two character shard directory is
  part of the cross implementation contract, not an implementation detail.
* **S3 layout: one bucket per volume (`mcpfs-{project_id}` by default), object
  key = the sha256**, path style addressing forced because MinIO requires it.
* **Git objects share the bucket** under the key `git:{sha}`, holding the
  canonical `{type} {len}\0{payload}` bytes. A sha256 is plain hex, so the prefix
  makes a collision impossible. See `.agent_docs/git.md`.
* **Orphan sweep, once a day per project (SPEC-0034, 2026-10-16).** A crash or
  database error between a blob's write and its metadata transaction can leave
  bytes nothing references. The purge cycle lists each project's bucket and
  deletes, strictly older than 24 hours, a content blob with no `blob_refs`
  row, an `export:{token}` zip with no `export_links` row, and a local
  `.tmp-*` leftover; `git:` keys are excluded on purpose (git's own
  reachability rules apply). Each candidate is re-checked, age and reference,
  right before its delete, since a write can land between the listing and the
  delete. A circuit breaker holds back every content blob of a project (export
  zips and temp files are unaffected) when the qualifying count exceeds
  max(1, floor(10% of that project's listed content blobs)), counted in
  `CycleSummary::orphan_breakers_tripped`. The schedule (last sweep per
  project) lives in memory on `StoreManager`, so it resets with the process.

## Project usage (SPEC-0020, 2026-10-12)

`volume_usage` (`volume_id` PK, `used_bytes`) holds a project's usage: the sum of the
declared sizes of its files, trashed files included (trash is a rename), each path once even
when content is shared. `put_file` adds `size - old_size`, `delete_file` subtracts the size,
`remove_subtree` the sum of its files, each in its own node transaction; `rename` changes
nothing. Export zips, git objects and artifact blobs are not nodes, so not usage.

* **Derived data.** A missing row (a volume written before the table existed) is rebuilt by
  `MetaBackend::used_bytes` from `SUM(size)` over the volume's files, in one transaction, at
  `RelationalMetaStore::open` and on read. An update that finds no row is a no-op: the
  backfill reads `nodes`, which already holds it. Dropping the table loses nothing.
* **Read.** `quota::used_bytes` takes the volume's admission lock (`VolumeClient::admission`,
  one tokio mutex per cached client) then reads one row. `admin.get_project_quota` and
  `GET /api/admin/projects/{project_id}/quota` call `tools::admin::get_project_quota`.
* **Disk usage read (2026-10-22, SPEC-0047).** `admin.get_project_disk_usage` and `GET
  /api/admin/projects/{project_id}/disk-usage` call `tools::admin::get_project_disk_usage`:
  cap check, `reader_project` gate (shared with the quota read), trash row backfill, then
  `quota::disk_usage`, which holds `VolumeClient::admission` while it reads the counter and
  runs `RelationalMetaStore::largest_files` (count, cutoff size, rows at least that big, all in
  one transaction). The order of equal sizes is decided in Rust (UTF-8 byte order), because
  text collation differs per engine. A trashed file's original path is its flat trash entry
  `/<trash_dir>/<stamp>__<flat>` row plus the rest of the path. No index: top N scan,
  O(n log k). The default trash directory is `.mcp_trash`.
* **Admission (2026-10-12, US-0020).** Every growing operation goes through
  `quota::admit*`: take `VolumeClient::admission`, compute the net change from the current
  sizes, check the project limit, THEN charge the session budget, and hold the lock until
  the last write landed, so two racing writes that fit alone but not together see each
  other (one refused) and usage never overshoots. A multi file operation (folder copy,
  extraction, every git apply) is admitted once for its total, all or nothing. Moves,
  trash, trash restore, git aborts and receive pack never call `admit` (DEC-009): they add
  no project bytes. A limit lowered below usage removes nothing; every positive delta is
  then refused. Fail closed: an error reading the limit or the counter refuses the
  operation rather than letting it through unchecked.
* **Gap.** Two processes sharing one PostgreSQL or SQL Server database can lose an update
  only during the very first backfill of a volume, if the other process writes between the
  SUM and the insert; the in process lock does not cross processes.

## Document artifacts (SPEC-0019, 2026-10-10)

The images and tables a conversion keeps live in four meta tables, every row keyed by
`volume_id`, never in `nodes`, so no file enumerator sees them (`storage/meta.rs`,
`docs/artifacts/`):

| Table | Role |
|---|---|
| `doc_artifact_current` | `(volume_id, path)` to the current `set_id` plus the `node_rev` it was built from |
| `doc_artifact_sets` | one row per set: `path`, `node_rev`, conversion text blob `text_sha`, `report`, `superseded_at` |
| `doc_images` | `seq`, caption, page, format, picture `sha256` (a counted blob ref), line position |
| `doc_tables` | `seq`, caption, row/col counts, CSV quality, cells as JSON, line position |

* **Validity token.** `nodes.rev` is a uuid restamped on every content write and move. A
  reader joins the pointer to `nodes` on `rev = node_rev`; a mismatch is `No artifacts`, so a
  changed document loses its set at once, with no write path touching artifact tables.
* **Commit.** `capture::PendingSet` is weighed and charged with the sibling before any write
  (one quota charge). The commit is one transaction: refuse when the node rev moved
  (`Document changed during conversion`), mark the previous set superseded, insert the new
  set and its rows, replace the pointer. A reader that resolved the old set keeps reading it.
* **Per path lock.** `lock_artifacts(path)` orders overlapping conversions of one document,
  so the one committing last also wrote the sibling. Ordinary writers take no lock.
* **Sweep.** `purge::run_cycle` calls `sweep_stale_artifact_sets`: a set superseded for more
  than 600 s, or whose `node_rev` no longer matches its node, loses its rows and blob refs in
  one transaction; the blobs reaching refcount 0 are deleted after.
* Converter limitation: the real `doc-convert` writes no `artifacts.json`, so its images carry no page and no caption failure marker; it emits PNG only and deduplicates identical figures, so one picture referenced twice is one image.

## The relational layer

Every store speaks `storage::rel::RelationalDb` rather than a driver, so the metadata
tree, the ACL registry, the git index and the OAuth store each run on SQLite (default),
PostgreSQL or SQL Server, chosen per store in the config. Full reference in
[`backends.md`](backends.md).

```text
RelationalMetaStore  RelationalAdminStore  RelationalGitDb  RelationalOAuthPersistence
         |                   |                   |                    |
         +---------+---------+---------+---------+--------------------+
                   |  RelationalDb (dyn) + Dialect
     +-------------+--------------+----------------+
     |                            |                |
SqliteRelationalDb        PgRelationalDb    MssqlRelationalDb
(serialized, actor)       (sqlx pool)       (bb8 + tiberius-ng)
```

Three properties matter above the layer:

* **A transaction is an owned handle.** `db.begin()` returns a `Box<dyn RelationalTx>`;
  `commit()` consumes it and a drop rolls back. Not a closure, because a closure cannot
  carry a value out and is unusable across these call sites.
* **`run_retrying` is opt in and asserts idempotence.** It re runs the whole closure on a
  transient failure (up to `MAX_TX_ATTEMPTS`, 3). Work that touches state outside the
  transaction must use `begin()` instead. See `backends.md`.
* **`volume_id` scopes every row.** One shared PostgreSQL or SQL Server database holds
  every volume, so `volume_id` is the first key column of `nodes`, `blob_refs` and the
  `git_*` tables and belongs in every `WHERE` clause. Under SQLite it is constant per
  file, so that deployment keeps one file per volume.

`storage::RelationalRegistry` opens one pool per distinct backend, dsn and schema triple
and shares it process wide, so pointing all four stores at one dsn uses one pool. SQLite
is excluded from that sharing: each file keeps its own serialized connection.

## Write, read, delete end to end

Write (`fs.write` and every edit tool through `fs_ops::commit`):

1. Normalize the path, then the no clobber check (`ERR_NO_CLOBBER` when the file
   exists and `overwrite` is false).
2. If the file exists: read guard (`ERR_EDIT_WITHOUT_PRIOR_READ`), then compute
   the unified diff that the result carries.
3. `quota::admit_file` (SPEC-0020, 2026-10-12): under the volume's admission
   lock, refuse with `ERR_PROJECT_QUOTA_EXCEEDED` (HTTP 507) when the net change
   (new size minus the path's current size) would push `volume_usage` over the
   project limit, then charge the session write quota (`ERR_WRITE_QUOTA_EXCEEDED`).
   Project check first, so a refusal charges no session budget. The lock is held
   until the write landed. The limit is read through the admin store attached
   with `SafetyManager::attach_limits` (composition root); unattached means no limit.
4. `create_parents` walks the parent chain, inserting missing directories.
5. `VolumeClient::write_bytes_atomic`: hash the bytes, `blob.put` (a temp file
   plus rename locally, so a reader never sees a partial blob; a put of an
   existing sha is a no op), then `meta.put_file`, then GC the sha that the
   overwrite orphaned.
6. Record the path as read (a fresh write satisfies the guard for a follow up
   edit) and append an audit entry.

Read (`fs.read` and friends): `meta.get` for the node, then `blob.get` with an
optional byte range. A NULL sha yields empty content. Text reads go through
`String::from_utf8_lossy`, so a binary file never fails a read, it comes back
lossy (use `fs.read_bytes` for exact bytes).

Delete (`fs_ops::delete_path`): a directory needs `recursive`. Soft delete is the
default: the node is renamed to `/{safety.trash_dir}/{epoch_ms}__{flattened
path}`, which keeps the blob and its refcount untouched. A hard delete
(`trash=false`) requires `safety.allow_hard_delete` on the server, otherwise
`ERR_NOT_SUPPORTED`; it removes the nodes and deletes every blob whose refcount
reached 0.

Soft delete and the purge sweep move the node and write its `trash_entries` row in ONE
transaction (`MetaBackend::rename_into_trash`, SPEC-0044, 2026-10-19), so a crash never leaves a
trashed file untracked; `rename_with_collision_retry` calls it once per `dst`/`dst~n` attempt and a
refused attempt rolls back both. `record_trash_entry` is `#[cfg(test)]` (fixtures only) and a
structural test (DT-001) fails if production code calls it. The lazy backfill (FR-NEW-011) remains
for legacy untracked nodes.

## Project removal (SPEC-0024, 2026-10-13)

`purge::remove_project` is the ONE permanent removal, called by `admin.delete_project`
(`mcp/server.rs`) and by the grace period purge (`purge::sweep_grace_period`, from the
background loop in `app.rs` and the `purge` CLI verb). Steps, in order:

1. `StoreManager::teardown_volume`: every metadata row of the volume (US-0025).
2. `SearchBackend::delete_all(project_id)` when a search backend is configured: drops the
   Tantivy directory and its cache entry, the cached sqlite-vec rows, the PostgreSQL
   `search_fts`/`search_chunks` rows. Without it a project recreated under the same id
   found the old content through `search.query` (CWE-226).
3. `GitRepoStore::purge_repo` when `git.enabled`.
4. `AdminBackend::delete_project`, last, so a failure at any step leaves the project
   listed and deletable again; every step is idempotent, a retry completes it.

Never add a store wipe to one caller only: add it to `remove_project`. Soft delete
(`soft_delete_project`) wipes nothing, so `admin.undelete_project` restores the trash,
artifacts and index unchanged. The `purge` CLI verb builds its search backend with
`search::build_backend` for the grace sweep. Tests: `tools/project_removal_e2e.rs`.

## Request lifecycle

| Step | Where | Failure |
|---|---|---|
| Bearer extraction | `identity.rs`, configured header then `Authorization`, also Basic with the token as password | `ERR_UNAUTHENTICATED` |
| RS256 verification | signature, `iss`, `exp`/`nbf`, 30s leeway, identity from `username_claim`, lowercased | HTTP 401 JSON `{error, detail}` |
| Membership gate | `AppState::authorize` -> `AdminBackend::require_member` | `ERR_PROJECT_NOT_FOUND` (404) or `ERR_FORBIDDEN` (403) |
| Tool dispatch | `mcp::server::McpServer`'s `rmcp` tool router (`#[tool_router]`/`#[tool]`) | unknown tool: JSON-RPC `-32602` |
| Argument parsing | `rmcp`'s `schemars`-derived `Parameters<T>` structs, one per tool, deserialized straight from the JSON-RPC params; every `#[tool]` method passes typed fields directly into the engine function it calls (no `Args`/JSON accessor layer in production) | `ERR_INVALID_ARGUMENT` (400) |

The MCP endpoint is the only guarded route on the JSON-RPC side; `/health` and
the OpenAPI pair are public by design. The REST plane verifies the bearer per
request in `dataplane::guarded`, and the git routes have their own gate (see
`.agent_docs/git.md`). A tool failure is a *result* carrying `isError`, not a
JSON-RPC error: only an unknown tool or an unknown method is a protocol error.

**Served tool set (2026-10-14, SPEC-0027 US-0030).** `McpServer::new` removes
every `git.*` route (`git.auth*` and `git.pr_*` included) when `git.enabled` is
false and every `search.*` route when `search.enabled` is false, so `tools/list`
omits them and `tools/call` answers `-32602` `tool not found` before any handler
runs. Membership never gated git on its own (CWE-284: any member could clone and
push with git off). Counts: 61 by default, 110 with git, 114 with git and
search. The static `McpServer::tool_router()` is still the full catalog the
golden contract and `/api/swagger.json` read; do not filter it. A test that
calls a git or search tool over MCP must enable the flag in its config.
Tests: `tools/tool_surface_e2e.rs`.

**MCP transport (rmcp 3.5.0, SPEC-0013).** `app::build` mounts `rmcp`'s
`StreamableHttpService` over `mcp::server::McpServer` at `mcp_path`, configured
with `LocalSessionManager` + `legacy_session_mode: true`. Two real, observed
behaviors to know before touching this:
* `initialize` is genuinely mandatory. A bare `tools/call`/`tools/list` with no
  prior `initialize` on a fresh connection gets **HTTP 422**, plain-text body
  `"Unexpected message, expect initialize request"` — not a JSON-RPC envelope.
  This is rmcp's own behavior under this session-manager pairing, empirically
  verified against the real crate source (not assumed from docs); see the
  SPEC-0013 drift directory for the full investigation.
* Once initialized, a plain tool call's response stays **`text/event-stream`**
  (SSE), same as before the migration — `rmcp`'s JSON-default framing mode is
  only reachable in a *stateless* configuration incompatible with mandatory
  `initialize`, so that particular declared break never materializes in this
  server's actual shipped behavior.
* `crates/agent/src/mcp.rs`'s client performs `initialize` + `notifications/
  initialized` before its first real call, matching this.

## Safety contract (`safety.rs`)

Session state is in memory, keyed `(person, project_id)`, so it resets on restart
and never leaks across callers.

| Rail | Behaviour |
|---|---|
| Path normalization | NUL byte rejected; a relative path becomes absolute; `..` is normalized away so traversal is contained rather than rejected (`/../../etc/passwd` becomes `/etc/passwd`, still inside the volume) |
| Read before write | an edit of an existing file needs a read of that exact path in this session; a write records the read; disabled with `safety.read_guard: false` |
| Write quota | `safety.write_quota_bytes` per session, charged before the write; a rejected write consumes nothing |
| Audit | ring buffer of 500 entries per session (`timestamp`, `op`, `path`, `detail`), served by `fs.audit_log` |
| Trash | `/{trash_dir}/{epoch_ms}__{path with / replaced by __}` |

## Concurrency

* **SQLite: one connection per database**, wrapped in a mutex, opened with
  `journal_mode=WAL`, `busy_timeout=5000`, `foreign_keys=ON`. Every access runs
  in a transaction (commit on `Ok`, rollback on `Err`) inside
  `tokio::task::spawn_blocking`, so a request thread never blocks on the db and
  "database is locked" cannot happen from within the process. A transaction is driven
  by a short lived actor holding the lock for its whole life, because a
  `rusqlite::Transaction` borrows its `Connection` and a `MutexGuard` is not `Send`, so
  it cannot be held across an `.await`.
* **PostgreSQL and SQL Server: a real connection pool** (`pool.max_connections`,
  `pool.acquire_timeout_secs`), so writes run concurrently instead of serialized. That
  makes two failures newly possible which SQLite could not produce: a serialization
  conflict and a deadlock victim. Both are marked `retryable` and are re run by
  `rel::run_retrying`, which is why work wrapped in it must be idempotent.
* **`StoreManager` caches one `VolumeClient` per project** behind a tokio mutex;
  the first access provisions the blob bucket.
* **Git**: one `GitRepoEntry` per project, `git2::Repository` behind a mutex
  because it is `Send` but not `Sync`, plus a dedicated `write_lock` held for a
  whole push. libgit2 work runs on the blocking pool.
* The safety manager uses a plain `std::sync::Mutex`: the critical sections are
  pure map updates with no await inside.

## Error logging split

`errors.rs` maps each `ERR_*` to an HTTP status; `logging.rs` uses that mapping as
the classifier. A `ToolError` below 500 (unauthenticated, forbidden, not found,
project not found, no clobber, out of bounds, invalid argument) is an expected
client caused outcome: logged once at INFO, concise, no context dump. Everything
else, including `ERR_INTERNAL_ERROR` and anything that is not a `ToolError`,
stays at ERROR with the full message. A misbehaving client therefore cannot flood
the log, and an ERROR line is always worth reading. Logs go to stderr so stdout
stays usable for `mcp-fs token`.

Every `ERR_*` code has an explicit HTTP status (see `ToolError::http_status`). The
reference mapped six codes and defaulted the rest to a generic 400, so a spent
quota, an edit without a prior read, an ambiguous match and an unsupported format
were indistinguishable by status. Here they are 429, 428, 409 and 501 respectively,
and `ToolError::is_client_error` tells the logging layer whether a failure was the
caller's fault, so a 4xx stays concise and monitoring is not paged for it.

### Transient database failures

A remote database adds failure modes an in process SQLite call could not have: a
serialization conflict, a deadlock victim, a pool timeout and a dropped connection. These
carry no new `ERR_*` code, so the code list did not grow for them (it is 16 today, after
`ERR_PASSWORD_REQUIRED` and `ERR_PROJECT_QUOTA_EXCEEDED`). Instead `ToolError` has a
`retryable` flag, set by `mark_retryable()` at the driver boundary, and
`rel::run_retrying` is the only thing that reads it.

The reason it is a flag and not a code: whether a failure is worth retrying is orthogonal
to what a client should be told. A caller still sees `ERR_INTERNAL_ERROR`, because a
conflict the server already retried three times is a server problem, not something the
caller can fix by changing the request. The message names the real cause, so a pool
timeout never reads as a generic SQLite failure.