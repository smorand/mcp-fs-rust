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
