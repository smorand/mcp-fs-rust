# Search Tools

The `search.*` family (4 tools: `search.index`, `search.query`, `search.delete`,
`search.status`) adds BM25 full-text search and optional RAG vector search to every
volume. They are registered only when `search.enabled: true` in the server config.

## Config keys

```yaml
search:
  enabled: false        # master switch; false = no tools registered
  mode: bm25            # bm25 | rag | both
  tantivy_dir: state/search  # base dir for Tantivy on-disk indexes (SQLite only)

  embedding:
    endpoint: ""        # OpenAI-compatible /v1/embeddings URL (required for rag/both)
    model: text-embedding-3-small
    api_key_env: MCPFS_EMBEDDING_KEY  # env var holding the Bearer key
    dimensions: 1536    # must match the model; frozen after first index call

  reranking:
    enabled: false      # set true to call the rerank endpoint on search.query
    endpoint: ""        # Cohere/Jina rerank URL
    model: ""
    api_key_env: MCPFS_RERANK_KEY
    top_n: 10           # how many chunks to send to the reranker
```

## Per-project index mode

`search.mode` above is the SERVER mode: which engine is wired. Whether a given
project's content reaches that engine is a per-project property, `index_mode`, stored
on the `project` row and managed by two tools:

| tool | gate | returns |
|---|---|---|
| `admin.set_index_mode` | owner or platform admin | `project_id`, `index_mode`, `previous_mode`, `reindex_started` |
| `admin.get_index_mode` | member, or platform admin | `project_id`, `index_mode` |

Values: `none` (the default for every project, including every project that existed
before the column shipped), `bm25`, `rag`, `both`. `rag` and `both` are refused with
`ERR_INVALID_ARGUMENT` when `search.embedding.endpoint` is empty; any value is refused
with `ERR_NOT_SUPPORTED` when `search.enabled` is false; an unknown value is
`ERR_INVALID_ARGUMENT`.

### Transitions

| From \ To | none | bm25 | rag | both |
|---|---|---|---|---|
| **none** | no-op | wipe + full index | wipe + full index | wipe + full index |
| **bm25** | wipe | no-op | wipe + full index | wipe + full index |
| **rag** | wipe | wipe + full index | no-op | wipe + full index |
| **both** | wipe | wipe + full index | wipe + full index | no-op |

"Wipe" is `SearchBackend::delete_all(volume_id)`, scoped to the one volume, and it is
finished before the tool answers. "Full index" is a detached `tokio` task walking every
file of the volume, so a `reindex_started: true` answer means a pass is RUNNING, not
that the volume is searchable. Poll `search.status` and watch `bm25_docs` /
`vector_chunks` settle. A same-mode call is a real no-op: it neither wipes nor rebuilds,
which is why it reports `reindex_started: false`.

### Auto indexing

While a project's mode is active, every write reaches the index without a
`search.index` call: `fs.write`, `fs.append`, `fs.write_bytes`, `fs.write_docx`,
`fs.edit`, `fs.multi_edit`, `fs.search_replace`, `fs.insert_at_line`, `fs.apply_patch`
and the REST `POST /api/fs/{mount}/upload`. `fs.delete` removes the deleted path, and
every file underneath it for a recursive delete. `fs.move` and `fs.copy` follow the
bytes (see below). A `dry_run` edit writes nothing and so
indexes nothing. Files that are not valid UTF-8 are skipped silently, which covers
binary uploads and the `.docx` that `fs.write_docx` produces.

The work is fire and forget (`search/indexer.rs`): the write returns as soon as the
bytes are committed, and the index call runs on a detached task whose failures are a
WARN log, never an error to the caller. **Trade-off**: an embedding round trip is
50 to 300 ms, so a caller that writes and immediately queries can miss its own write.
There is no read-your-write guarantee on the index. A caller that needs one must poll
`search.status` or call `search.index` synchronously.

#### Move and copy

`fs.move` drops the index entries of the source and indexes the destination by reading
it back from the volume; `fs.copy` indexes the destination and leaves the source's
entries alone, because a copy leaves the source's bytes alone. Both follow the WHOLE
subtree, not just the named path, so a recursive move or copy of a tree moves or
duplicates every entry underneath it. The REST `POST /api/fs/{mount}/move` and
`/copy` call the same two helpers (`search/indexer.rs`), so the two doors cannot drift.

**Ordering matters on a move**: the source paths are enumerated BEFORE the rename and
the destination is walked after it. Once the rename has happened there is no source
tree left to walk, so entries collected too late would stay in the index forever with
no path to enumerate them by. This is the same enumerate-first rule a recursive
`fs.delete` follows, and the bug it was written to prevent.

A move or copy that FAILS leaves the index untouched: the hooks run only on the success
path, after the engine has returned. Non UTF-8 destinations are skipped silently, and
directories carry no entries of their own.

When the mode is `none`, a write costs one extra `project` row read and nothing else.
Chunking uses the `search.index` defaults (1000 / 100); there is no per-project chunk
configuration.

The mode decides WHETHER a project is indexed, not by which engine: the backend is
process wide and serves whatever `search.mode` configured. Setting `bm25` on a server
running `search.mode: rag` indexes into the vector store.

### Crash mid-reindex

The wipe commits before the rebuild starts, so a server that dies during the initial
full index leaves the project with an active mode and a partial or empty index. The
state is consistent, never corrupt, but nothing restarts the pass: call
`admin.set_index_mode` with a DIFFERENT mode and back, or `search.index` with
`recursive: true`, to rebuild. Same for the storage schema: `index_mode` is written
before the wipe, so the failure mode is a stale index under a correct mode rather than
a wiped index under a mode nothing will rebuild.

## Modes

| mode  | BM25 engine    | Vector engine  | Merge | Reranking |
|-------|----------------|----------------|-------|-----------|
| bm25  | Tantivy        | none           | none  | no        |
| rag   | none           | sqlite-vec     | none  | optional  |
| both  | Tantivy        | sqlite-vec     | RRF   | optional  |

The `mode` param on `search.query` overrides the server default per call.

Requesting a mode that requires a capability that is not provisioned (for example
`rag` with an empty `embedding.endpoint`) returns `ERR_NOT_SUPPORTED`.

## PostgreSQL (future)

When `infra.meta.backend = postgres`, BM25 would use `tsvector` + GIN and vectors
would use `pgvector`. The `CREATE EXTENSION IF NOT EXISTS vector` DDL runs in the
pool's `after_connect` hook when the `rag` feature is compiled in. The `VECTOR(N)`
column type renders via `ColumnType::Vector(u32)` in `storage/rel/dialect.rs`.

This path is wired in the dialect and schema layers but the PostgreSQL search backend
itself is not yet implemented. The current implementation uses Tantivy (BM25) and
sqlite-vec (RAG) regardless of the infra backend.

## SQLite: Tantivy BM25

Tantivy index directories live at `{tantivy_dir}/{volume_id}/`. The index is:

- **Ephemeral**: a pod restart or volume remount loses the index. Must re-index after
  restart via `search.index`.
- **Warm check**: `search.status` reports `bm25_warm: false` when the directory is
  absent. `search.query` adds a `warning` field in this case.
- **Idempotent**: `search.index` deletes existing chunks for a path before inserting.
- **Rebuildable**: call `search.index` with `recursive: true` to re-index a tree.

## SQLite: sqlite-vec

sqlite-vec 0.1.9 (pinned, pre-v1) provides `vec0` virtual tables for vector KNN. The
extension is registered once at process start via `sqlite3_auto_extension` when the
`rag` cargo feature is on. The extension is statically linked via the `cc` crate,
so no system library is needed (same posture as `rusqlite` with `bundled`).

Vectors are stored in a `vec0` virtual table alongside a `search_vec_meta` companion
table. The DDL uses the `SchemaSet::virtual_tables` field in the schema renderer.

## SQL Server

All search modes return `ERR_NOT_SUPPORTED` on SQL Server. The dialect types
(`Tsvector`, `Vector(N)`) degrade to `NVARCHAR(MAX)` so the schema applies cleanly,
but the tools reject the call at runtime.

## Chunking

Fixed character window, configurable per call:

- `chunk_size` (default 1000 chars): maximum chunk length
- `chunk_overlap` (default 100 chars): overlap between consecutive chunks

The chunker tries to break at sentence boundaries (`.`, newline, space) within a
50-char tolerance so chunks do not end mid-word. See `search/chunker.rs`.

## RRF Fusion

`mode=both` merges BM25 and vector results using Reciprocal Rank Fusion with k=60.
A document appearing in both lists scores higher than one in only one list.
The merge is deterministic: same inputs always produce the same output.

## Reranking

When `search.reranking.enabled: true` and `rerank: true` (the default on
`search.query`), the results are re-scored by the configured endpoint using the
Cohere/Jina rerank API shape:

- POST `{"model": "...", "query": "...", "documents": ["text1", ...]}`
- Response `{"results": [{"index": 0, "relevance_score": 0.9}, ...]}`

A reranking failure is non-fatal: the original order is returned with a log warning.

## Index lifecycle

1. `search.index` (required before any query)
2. `search.query` (BM25, RAG, or both)
3. `search.delete` (removes a path)
4. `search.status` (reports `bm25_docs`, `vector_chunks`, `bm25_warm`, `mode`)

`search.index` is idempotent: calling it twice on the same path replaces the existing
chunks. Files that cannot be read as text are silently skipped (counted in `skipped`).

## Authorization

All `search.*` tools call `state.authorize(mount_id, person)` before any storage
access, the same gate as `fs.*`. Platform admins do not get implicit search access;
they must be a project member.

## Testing

Use the `testkit::harness_with_search` helper in unit tests. It injects a real
`TantivyBm25Backend` so tests run offline without any embedding server. For RAG/both
mode integration tests, use `scripts/search_embedding_fake.py` as a stub server.

## Risks

1. **sqlite-vec is pre-v1** (pinned at 0.1.9). Breaking changes expected before 1.0.
   Blast radius is contained to `search/vector_sqlite.rs`.

2. **Tantivy index is ephemeral** on SQLite. Re-index after any pod restart. The
   `search.status` tool and the `warning` field on `search.query` signal this.

3. **Embedding dimension freeze**: once vectors are stored at dimension N, changing
   `search.embedding.dimensions` breaks all stored vectors silently. Delete chunks
   and re-index after any dimension change.

4. **`delete_all` on Tantivy removes the index directory** (`{tantivy_dir}/{volume_id}/`)
   rather than deleting term by term, which avoids segment fragmentation and is safe
   because the directory is always rebuildable. The cached open index is evicted with
   it, so the next write reopens a fresh one.

5. **A move or copy with `overwrite: true` onto an existing tree can leave stale
   entries.** The destination's own files are re-indexed, but a file that existed only
   under the old destination and is not in the new one keeps its entry. Re-run
   `search.index` after an overwriting reorganisation.
