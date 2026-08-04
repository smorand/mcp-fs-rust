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
| mcp::registry  |     | api::dataplane       |     | git::http          |
| 55 tools       |     | 36 routes            |     | smart protocol v0  |
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
        | SqliteMetaStore | LocalBlobStore / S3BlobStore | SqliteAdminStore|
        +------------------------------------------------------------------+
```

## Storage model

A volume is a pair: a metadata tree in SQLite and content addressed bytes in a
blob store. The two are bound by `storage::volume::VolumeClient`.

| Table | Where | Columns |
|---|---|---|
| `nodes` | `state/volumes/{project}.db` | `path` (PK), `parent`, `name`, `kind` (dir/file), `size`, `mode`, `mtime`, `ctime`, `atime`, `sha256` |
| `blob_refs` | same db | `sha256` (PK), `refcount`, `size` |
| `project`, `project_member` | `state/admin.db` | ACL registry, all identities normalized caseless |

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

## Write, read, delete end to end

Write (`fs.write` and every edit tool through `fs_ops::commit`):

1. Normalize the path, then the no clobber check (`ERR_NO_CLOBBER` when the file
   exists and `overwrite` is false).
2. If the file exists: read guard (`ERR_EDIT_WITHOUT_PRIOR_READ`), then compute
   the unified diff that the result carries.
3. `create_parents` walks the parent chain, inserting missing directories.
4. Charge the session write quota (`ERR_WRITE_QUOTA_EXCEEDED`) **before** any
   byte is written.
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

## Request lifecycle

| Step | Where | Failure |
|---|---|---|
| Bearer extraction | `identity.rs`, configured header then `Authorization`, also Basic with the token as password | `ERR_UNAUTHENTICATED` |
| RS256 verification | signature, `iss`, `exp`/`nbf`, 30s leeway, identity from `username_claim`, lowercased | HTTP 401 JSON `{error, detail}` |
| Membership gate | `AppState::authorize` -> `AdminBackend::require_member` | `ERR_PROJECT_NOT_FOUND` (404) or `ERR_FORBIDDEN` (403) |
| Tool dispatch | `mcp::registry`, exact name then a dot/underscore tolerant match | unknown tool: JSON-RPC `-32602` |
| Argument parsing | `mcp::args`, tolerant accessors (string arrays accept an array, a bare string, or a comma separated string) | `ERR_INVALID_ARGUMENT` (400) |

The MCP endpoint is the only guarded route on the JSON-RPC side; `/health` and
the OpenAPI pair are public by design. The REST plane verifies the bearer per
request in `dataplane::guarded`, and the git routes have their own gate (see
`.agent_docs/git.md`). A tool failure is a *result* carrying `isError`, not a
JSON-RPC error: only an unknown tool or an unknown method is a protocol error.

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

* **One SQLite connection per database**, wrapped in a mutex, opened with
  `journal_mode=WAL`, `busy_timeout=5000`, `foreign_keys=ON`. Every access runs
  in a transaction (commit on `Ok`, rollback on `Err`) inside
  `tokio::task::spawn_blocking`, so a request thread never blocks on the db and
  "database is locked" cannot happen from within the process.
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