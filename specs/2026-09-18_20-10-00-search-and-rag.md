# mcp-fs Search and RAG — Specification Document

> Generated on: 2026-09-18
> Project: mcp-fs (Rust)
> Version: 1.0
> Status: Draft
> Type: Evolution Specification (retro-specification of shipped behaviour)

**ID convention.** S7 of eight. S7 owns the `7xx` block: `SC-7xx`, `FR-7xx`, `E2E-7xx`, `DEC-7xx`, `EXC-7xx`.

## 1. Executive Summary

This document specifies search over volume content: **four `search.*` tools**, **four backends** behind one trait, **automatic indexing** driven by a per-project mode, and the hybrid retrieval path that merges full-text and vector results.

Three rules define the layer's behaviour and they are all deliberate trades. **A write never waits for the index**: indexing is handed to a detached task, so a slow or absent embedding endpoint costs a write nothing, at the price of the index lagging the volume by one backend call. **An index failure is a log line, not an error**: the caller already committed bytes, and refusing the write afterwards would be a lie. And **a project's mode decides whether its content is indexed, not which engine indexes it**: the engine is a server-wide configuration choice.

The backend set is chosen by combining `search.mode` with `infra.meta.backend`: Tantivy BM25 on SQLite, PostgreSQL tsvector, pgvector, sqlite-vec. Vector search needs the `rag` Cargo feature, which implies `postgres`.

Retro-specification of shipped behaviour; every claim carries a `file:LINE` citation.

## 2. Current State Analysis

### 2.1 Project Overview

`search/` is 6114 lines, of which `e2e.rs` is 2960: the test suite is roughly half the subsystem, which is unusual and worth noting because it means much of what this document specifies is already pinned by tests. The rest is `indexer.rs` (700), `vector_sqlite.rs` (523), `bm25_sqlite.rs` (451), `vector_pg.rs` (451), `bm25_pg.rs` (337), `mod.rs` (312), `fusion.rs` (114), `chunker.rs` (112), `rerank.rs` (93), `embedding.rs` (61).

### 2.2 Existing Specifications

- **S1**: `IndexMode` persistence and the two admin tools that set and read it (FR-030, FR-031), the membership gate the search tools reuse, and `volume_id` scoping.
- **S2**: the write and delete operations whose side effects trigger indexing.
- **S3**: REST upload, another write path that must index identically.
- **S4**: the relational seam, the `Tsvector` and `Vector(n)` column types (FR-406), and the `rag` feature implying `postgres` (FR-410).
- **S5**: extraction, which produces the companion text that gets indexed.

### 2.3 Relevant Architecture

- **Trait**: `SearchBackend` (`search/mod.rs:56-95`), six methods; a combined "both" backend is assembled by the factory rather than by a wrapper type, so the trait stays flat.
- **Factory**: `build_backend` (`search/mod.rs`), keyed on `(search.mode, infra.meta.backend)`.
- **Auto indexing**: `indexer.rs`, with `AUTO_CHUNK_SIZE` 1000 and `AUTO_CHUNK_OVERLAP` 100 (`:32-34`), and hooks `after_write`, `after_write_reread`, `after_companion`, `after_patch`, `after_delete`, `after_delete_many` (`:182-267`).
- **Chunking**: `chunker.rs`, fixed windows with sentence-boundary tolerance.
- **Fusion**: `fusion.rs`, RRF with `K = 60` (`:14`).
- **Reranking**: `rerank.rs`, a Cohere/Jina-compatible HTTP call.
- **Tools**: `tools/search_semantic.rs`, four tools.

## 3. Scope

### 3.1 In Scope

- The four tools: `search.index`, `search.query`, `search.delete`, `search.status`.
- The `SearchBackend` trait and its four implementations.
- Backend selection from `search.mode` crossed with `infra.meta.backend`, and its feature gating.
- Chunking: window size, overlap, boundary tolerance.
- Auto indexing: the mode-to-operation mapping, the detached-task rule, the failure rule, and every write and delete hook.
- Mode changes: the wipe, the background rebuild, and their ordering relative to the stored mode.
- Hybrid retrieval: RRF merge and its determinism.
- Reranking: when it applies and what it calls.
- Embeddings: the OpenAI-compatible endpoint, model, dimension and key handling.
- `IndexStats` and what `search.status` reports.

### 3.2 Out of Scope (Non-Goals)

- `IndexMode` persistence and the two `admin.*` tools that manage it (S1 FR-030, FR-031). S7 specifies what a mode *does*.
- The write and delete operations themselves (S2, S3).
- Text extraction (S5); search indexes whatever text it is handed.
- The relational seam and the vector column types (S4).
- Any ranking quality claim. This document specifies mechanism, not relevance.

## 4. User Personas & Actors

| Actor | Description |
|---|---|
| **LLM agent** | Queries for content it cannot find by name. Hybrid search is what makes a volume searchable by meaning rather than by path. |
| **Project owner** | Chooses the project's index mode, and pays the rebuild cost when switching it on. |
| **Operator** | Decides whether search exists at all, which engine serves it, whether an embedding endpoint is available, and whether reranking is configured. |
| **Embedding provider** | An OpenAI-compatible endpoint. External, optional and potentially slow, which is why writes never wait for it. |
| **Reranker** | A Cohere or Jina-compatible endpoint. External and optional. |

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| **Index** | Stored chunks and their retrieval structures. | chunk, document, vector, tsvector |
| **Retrieval** | Turning a query into ranked results. | query, top_k, score, rank, RRF |
| **Auto indexing** | The side effects a mode attaches to writes and deletes. | IndexMode, hook, detached task |
| **Embedding** | Turning text into vectors through an external endpoint. | model, dimensions, API key |

A "document" in the Index context is one indexed chunk, not a file: `IndexStats.bm25_docs` counts chunks. A "mode" is overloaded and this spec distinguishes the **project index mode** (S1's `IndexMode`, whether content is indexed) from the **server query mode** (`search.mode`, which engine answers).

## 5. Usage Scenarios

### SC-701: Owner enables indexing for a project

**Actor:** Project owner
**Preconditions:** `search.enabled` true; for a vector mode, an embedding endpoint configured.
**Flow:**
1. Owner calls `admin.set_index_mode` with `bm25`, `rag` or `both` (S1 FR-030).
2. The new mode is stored **before** any index work.
3. The indexer reacts to the change: switching to an active mode starts a background full index of existing files; switching to `none` wipes the index immediately.
4. The tool reports `reindex_started`.
5. Owner polls `search.status` to watch the index fill.

**Postconditions:** the project's existing content is being indexed; new writes are indexed automatically from now on.
**Exceptions:**
- EXC-701a: `search.enabled` false → `ERR_NOT_SUPPORTED` naming the config key (S1 FR-030)
- EXC-701b: a vector mode with no embedding endpoint → `ERR_INVALID_ARGUMENT` at the gate
- EXC-701c: a crash between the stored mode and the wipe → the mode is recorded and the index stale, which one more `set_index_mode` repairs
- EXC-701d: `reindex_started` true means a background pass is running, **not** that the volume is searchable

### SC-702: Agent indexes a path explicitly

**Actor:** LLM agent
**Preconditions:** member; search enabled.
**Flow:**
1. Agent calls `search.index` with a path, optionally `recursive`, `chunk_size` (default 1000) and `chunk_overlap` (default 100).
2. The backend deletes any existing chunks for that path, then splits the text into overlapping windows and inserts them.
3. The tool reports how many chunks were inserted.

**Postconditions:** the path is searchable; re-indexing it is idempotent because the delete precedes the insert.
**Exceptions:**
- EXC-702a: a directory without `recursive` → only that path is considered
- EXC-702b: a binary or unreadable file → skipped rather than failing the call
- EXC-702c: an empty file → zero chunks, not an error

### SC-703: Agent searches and gets hybrid results

**Actor:** LLM agent
**Preconditions:** member; content indexed.
**Flow:**
1. Agent calls `search.query` with a query, optionally `mode`, `top_k` (default 10) and `rerank` (default true).
2. The mode defaults to the server's configured mode when the caller does not name one.
3. In `bm25` the full-text backend answers; in `rag` vector search answers; in `both` hybrid retrieval queries each and merges the two lists by RRF.
4. When reranking is configured and requested, the merged list is sent to the rerank endpoint and reordered.
5. Results carry `path`, `score`, `chunk` and `rank`.

**Postconditions:** the agent holds ranked chunks with the paths they came from.
**Exceptions:**
- EXC-703a: a vector mode with no embedding endpoint → refused
- EXC-703b: the rerank endpoint unreachable → results are returned unreranked rather than the query failing
- EXC-703c: nothing indexed → an empty result list, not an error
- EXC-703d: a document present in both result lists → ranked above one present in only one

### SC-704: Content is indexed automatically as it changes

**Actor:** LLM agent, implicitly
**Preconditions:** the project's mode is active.
**Flow:**
1. Agent writes a file through any surface: an `fs.*` tool, a REST upload, a patch, or a document companion.
2. The matching hook hands indexing to a **detached task** and returns immediately (`search/indexer.rs:8-12`).
3. The write's response is returned without waiting.
4. The chunks appear in the index shortly afterwards.
5. A delete removes the path's chunks the same way.

**Postconditions:** the index converges on the volume's content without any caller waiting for it.
**Exceptions:**
- EXC-704a: the embedding endpoint slow or absent → the write still succeeds at full speed
- EXC-704b: indexing fails → a log line, never an error to the caller, because the bytes are already committed (`search/indexer.rs:13-15`)
- EXC-704c: a caller that writes and immediately queries → may not see the new content, because the index lags by one backend call
- EXC-704d: the project's mode is `none` → no hook does anything

### SC-705: Operator picks an engine combination

**Actor:** Operator
**Preconditions:** a build whose features match the intended mode.
**Flow:**
1. Operator sets `search.mode` and `infra.meta.backend`.
2. The factory selects the backend pair from that combination.
3. A vector mode additionally requires an embedding endpoint, checked both at boot validation and at construction.
4. The server starts with a process-wide backend serving every project.

**Postconditions:** every project with an active mode is indexed by that engine.
**Exceptions:**
- EXC-705a: `bm25` with `postgres` and the `postgres` feature absent → `ERR_NOT_SUPPORTED` naming the feature
- EXC-705b: `rag` or `both` with an empty embedding endpoint → `ERR_INVALID_ARGUMENT`
- EXC-705c: `search.enabled` false → no backend is built and the four tools are not registered
- EXC-705d: an unknown mode string → rejected at boot validation

## 6. Functional Requirements

### Tools

#### FR-701 [EARS-U]: Four tools, registered only when search is enabled
> The server SHALL register `search.index`, `search.query`, `search.delete` and `search.status` when `search.enabled` is true, and SHALL register none of them otherwise.

- **Inputs:** `search.enabled`.
- **Outputs:** four tools, or none (`tools/all.rs:44-46`).
- **Business Rules:** every tool takes `mount_id` and authorizes through the membership gate before any storage access, exactly like the `fs.*` family (`tools/search_semantic.rs:4-6`).
- **Priority:** Must-have

#### FR-702 [EARS-E]: Explicit indexing
> WHEN `search.index` is called THE backend SHALL delete any existing chunks for the path, split its text into overlapping windows, insert them, and report the number inserted.

- **Inputs:** `path`, `recursive` default false, `chunk_size` default 1000, `chunk_overlap` default 100.
- **Outputs:** the inserted chunk count (`search/mod.rs:60-72`).
- **Business Rules:** delete-then-insert is what makes indexing idempotent, so re-indexing a path never duplicates chunks.
- **Priority:** Must-have

#### FR-703 [EARS-E]: Querying
> WHEN `search.query` is called THE server SHALL answer with ranked results carrying `path`, `score`, `chunk` and `rank`.

- **Inputs:** `query`, `mode` defaulting to the server's configured mode, `top_k` default 10, `rerank` default true.
- **Outputs:** `SearchResult` entries (`search/mod.rs:35-41`, `tools/search_semantic.rs:95-99`).
- **Business Rules:** an absent `mode` argument falls back to `search.mode` from config (`tools/search_semantic.rs:110`), so a caller need not know the deployment's engine.
- **Priority:** Must-have

#### FR-704 [EARS-E]: Deletion from the index
> WHEN `search.delete` is called THE backend SHALL remove every chunk for the path and report the count removed.

- **Inputs:** `path`, `recursive` default false.
- **Outputs:** the deleted chunk count (`search/mod.rs:74-76`).
- **Priority:** Must-have

#### FR-705 [EARS-E]: Status reporting
> WHEN `search.status` is called THE server SHALL report the BM25 chunk count, the vector chunk count, whether the BM25 index is warm, and the active mode.

- **Inputs:** `mount_id`.
- **Outputs:** `IndexStats` (`search/mod.rs:44-54`).
- **Business Rules:** `bm25_docs` counts **chunks**, not files. `vector_chunks` is 0 when RAG is not enabled. `bm25_warm` reports a warm index, meaning the Tantivy index directory exists and is non-empty. This is the tool an owner polls to watch a rebuild progress (S1 FR-030).
- **Priority:** Must-have

### The backend seam

#### FR-706 [EARS-U]: One trait, four implementations
> Every search engine SHALL implement `SearchBackend`, and the trait SHALL remain flat rather than gaining a combining wrapper type.

- **Inputs:** any search operation.
- **Outputs:** engine-independent results (`search/mod.rs:56-59`).
- **Business Rules:** the six operations are `index_path`, `delete_path`, `delete_all`, `query_bm25`, the vector query and the statistics call. A combined `both` backend is assembled by the factory, not by a wrapper, which is what keeps the trait from growing a mode parameter.
- **Priority:** Must-have

#### FR-707 [EARS-E]: Backend selection
> WHEN the backend is built THE factory SHALL select an implementation from the pair `(search.mode, infra.meta.backend)`.

- **Inputs:** the two config values.
- **Outputs:** the backend, or `None` when search is disabled (`search/mod.rs` `build_backend`).
- **Business Rules:** `bm25` with `postgres` selects the tsvector backend; `bm25` with anything else selects Tantivy; `rag` with `postgres` selects pgvector; `rag` on SQLite selects sqlite-vec. The Tantivy backend is always compiled; the others are feature-gated.
- **Priority:** Must-have

#### FR-708 [EARS-O]: Feature gating
> IF a selected backend's Cargo feature was not compiled in THEN the factory SHALL fail with `ERR_NOT_SUPPORTED` naming the required feature.

- **Inputs:** the selection and the compiled feature set.
- **Outputs:** the refusal, for example `search.mode=bm25 with backend=postgres requires the postgres feature`.
- **Business Rules:** vector backends need `rag`, which implies `postgres` (S4 FR-410). A default build supports `bm25` on SQLite and nothing else.
- **Priority:** Must-have

#### FR-709 [EARS-O]: Embedding endpoint is required for vector modes
> IF `search.mode` is `rag` or `both` and `search.embedding.endpoint` is empty THEN the factory SHALL fail with `ERR_INVALID_ARGUMENT`.

- **Inputs:** the mode and the endpoint.
- **Outputs:** the refusal (`search/mod.rs`, the guard before backend construction).
- **Business Rules:** the check is duplicated deliberately: boot validation performs it, and the factory performs it again so the error is clear when called from a test that skipped boot validation. A vector mode without an endpoint indexes nothing and reports success, which is why the same rule also gates `admin.set_index_mode` (S1 FR-030).
- **Priority:** Must-have

#### FR-710 [EARS-U]: The backend is process-wide
> The server SHALL build one search backend for the whole process, serving every project.

- **Inputs:** the server config.
- **Outputs:** the shared backend (`search/indexer.rs:16-18`).
- **Business Rules:** a project's index mode decides **whether** its content is indexed, never which engine indexes it. Per-project engine choice does not exist.
- **Priority:** Must-have

#### FR-711 [EARS-U]: Index rows are volume-scoped
> Every index operation SHALL be scoped by `volume_id`.

- **Inputs:** the volume id.
- **Outputs:** rows belonging to one volume (`search/mod.rs:60-79`).
- **Business Rules:** `delete_all` is the wipe half of a mode change, so it must be scoped to the one volume: another project's chunks live in the same tables (`search/mod.rs:77-79`). This is S1 FR-041 applied to the index.
- **Priority:** Must-have

### Chunking and retrieval

#### FR-712 [EARS-E]: Chunking with boundary tolerance
> WHEN text is chunked THE chunker SHALL produce windows of at most `size` characters overlapping by `overlap` characters, looking back up to 50 characters for a sentence boundary when the cut falls inside a word.

- **Inputs:** the text, `size`, `overlap`.
- **Outputs:** the chunk list (`search/chunker.rs:1-7`).
- **Business Rules:** boundary characters are `.`, newline and space. `overlap` is clamped below `size`, because an overlap equal to the size loops forever (`search/chunker.rs:15-16`). Empty text or a zero size yields no chunks. The overlap exists so an embedding captures cross-boundary context.
- **Priority:** Must-have

#### FR-713 [EARS-E]: Reciprocal rank fusion
> WHEN both result lists are available THE server SHALL merge them by RRF with k equal to 60, re-ranking from 1.

- **Inputs:** the BM25 and vector lists.
- **Outputs:** one merged list (`search/fusion.rs:1-20`).
- **Business Rules:** RRF with k=60 is parameter-insensitive over a wide range of corpus sizes and needs no calibration. A document appearing in both lists scores above one appearing in only one. **The merge is deterministic**: ties are broken by path lexicographically, so the output is stable across calls (`search/fusion.rs:7-8`).
- **Priority:** Must-have

#### FR-714 [EARS-O]: Reranking
> IF reranking is enabled, configured and requested THEN the server SHALL send the merged results to the rerank endpoint and return them in relevance order.

- **Inputs:** `rerank` default true; `search.reranking` config with `endpoint`, `model`, `api_key_env` and `top_n`.
- **Outputs:** the reordered list (`search/rerank.rs:1-6`).
- **Business Rules:** the request is `{"model","query","documents"}` and the response `{"results":[{"index","relevance_score"}]}`, a Cohere/Jina-compatible shape. `top_n` bounds how many results are sent. API keys are read from the environment variable named by `api_key_env`, never from the config file.
- **Priority:** Should-have

#### FR-715 [EARS-E]: Embedding calls
> WHEN vectors are needed THE server SHALL call the configured OpenAI-compatible `/v1/embeddings` endpoint with the configured model.

- **Inputs:** `endpoint`, `model`, `api_key_env`, `dimensions`.
- **Outputs:** the vectors (`config.rs:579-588`).
- **Business Rules:** `dimensions` must match the model's output, and it is the column width the vector type is declared with (S4 FR-406). An empty `api_key_env` means an unauthenticated endpoint, which is what a local embedding server needs.
- **Priority:** Must-have

### Auto indexing

#### FR-716 [EARS-U]: One place maps a mode to index operations
> The mapping from a project's index mode to index operations SHALL exist in exactly one module, used by every write path.

- **Inputs:** the project mode and the operation.
- **Outputs:** the index side effect (`search/indexer.rs:1-4`).
- **Business Rules:** the write tools, the delete tool, the REST upload and `admin.set_index_mode` all go through it, so every surface behaves the same. This is the search-side equivalent of S2 FR-201.
- **Priority:** Must-have

#### FR-717 [EARS-UB]: A write never waits for the index
> The server SHALL NOT block a write or a delete on indexing.

- **Inputs:** any write or delete.
- **Outputs:** the response, returned before indexing completes (`search/indexer.rs:8-12`).
- **Business Rules:** the hook hands the work to a detached task and returns, so a slow or absent embedding endpoint costs a write nothing. The consequence is stated rather than hidden: the index lags the volume by the duration of one backend call, which is visible to a caller that writes and queries back to back.
- **Priority:** Must-have

#### FR-718 [EARS-UB]: An index failure never fails the operation
> The server SHALL NOT turn an indexing failure into an error returned to the caller.

- **Inputs:** a failing index operation.
- **Outputs:** a log line (`search/indexer.rs:13-15`).
- **Business Rules:** the caller already committed bytes; refusing the write afterwards is a lie. This mirrors the document service rule in S5 FR-514, for the same reason.
- **Priority:** Must-have

#### FR-719 [EARS-E]: Every mutation path has a hook
> WHEN content is written, patched, companion-generated or deleted THE corresponding hook SHALL index or de-index it.

- **Inputs:** the mutation.
- **Outputs:** the index side effect (`search/indexer.rs:182-267`).
- **Business Rules:** the hooks are `after_write`, `after_write_reread` (for a write whose text the caller does not hold), `after_companion` (for S5's generated Markdown), `after_patch` (walking a patch report), `after_delete` and `after_delete_many`. A move is handled by de-indexing the displaced paths (`search/indexer.rs:298`).
- **Priority:** Must-have

#### FR-720 [EARS-U]: Auto-indexing chunk window
> Auto indexing SHALL use a chunk size of 1000 characters and an overlap of 100.

- **Inputs:** the indexed text.
- **Outputs:** the chunks (`search/indexer.rs:32-34`).
- **Business Rules:** the constants match the `search.index` tool's defaults, so explicit and automatic indexing produce the same chunking.
- **Priority:** Must-have

#### FR-721 [EARS-E]: Mode change side effects
> WHEN a project's index mode changes THE indexer SHALL wipe the existing index when moving to `none`, and SHALL start a background full index when moving to an active mode.

- **Inputs:** the old and new modes.
- **Outputs:** the wipe, or a started rebuild reported as `reindex_started` (`search/indexer.rs:54`).
- **Business Rules:** the wipe finishes before the tool answers; the rebuild does not. `reindex_started: true` therefore means a background pass is running, not that the volume is searchable. The mode is stored before either happens (S1 FR-030), so the recoverable failure is a stale index rather than a wiped one nothing rebuilds.
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
- Writes are never slowed by indexing (FR-717); the cost is moved off the request path entirely.
- `top_k` bounds result size; `top_n` bounds how much is sent to the reranker (FR-714).
- Chunk size and overlap bound embedding call volume: smaller chunks mean more vectors and more cost.
- A full rebuild after a mode change is a background pass whose duration is unbounded and unreported beyond `search.status` counts.

### 7.2 Security
- Every tool passes the membership gate before any storage access (FR-701).
- Index rows are volume-scoped, so one project's query cannot return another's chunks (FR-711).
- Embedding and rerank API keys are read from environment variables named in config, never stored in the config file (FR-714, FR-715).
- Indexed chunks are stored server-side and are returned only to members of the owning project.

### 7.3 Usability
- `mode` defaults to the server's configuration, so a caller need not know the deployment (FR-703).
- A vector mode without an endpoint is refused at the gate rather than silently indexing nothing (FR-709).
- `search.status` gives an owner a way to watch a rebuild (FR-705).
- A failing reranker degrades to unreranked results rather than failing the query (EXC-703b).

### 7.4 Reliability
- Indexing is idempotent by delete-then-insert (FR-702).
- The mode is stored before the index is touched, so the failure mode is repairable by repeating the call (FR-721).
- An index failure never corrupts or rejects a committed write (FR-718).
- RRF is deterministic, so identical inputs give identical output (FR-713).

### 7.5 Observability
Unchanged from S1 §7.5, with one consequence that matters here: because indexing failures are log lines (FR-718) and nothing counts them, a persistently failing embedding endpoint produces a silently stale index and no signal other than log volume. `search.status` shows chunk counts but nothing about failures. Recorded as TBD-701.

### 7.6 Deployment
- `search.enabled` off by default.
- `search.mode` is `bm25`, `rag` or `both`.
- `search.tantivy_dir` holds the on-disk BM25 indexes for the SQLite path.
- `search.embedding` needs an endpoint, a model and matching `dimensions`.
- `search.reranking` is off by default.
- Vector modes need the `rag` feature, which implies `postgres`.

### 7.7 Scalability
- Chunks multiply documents: a large file becomes many rows and many vectors.
- The Tantivy path keeps one on-disk index under `search.tantivy_dir`; the PostgreSQL paths keep rows in the shared database, scoped by `volume_id`.
- A full rebuild reads every file in a project, so enabling indexing on a large project is a heavy background operation with no throttle.
- Detached indexing tasks are unbounded in number: a burst of writes spawns a burst of tasks. Recorded as TBD-702.

## 8. Data Model

| Structure | Fields | Home |
|---|---|---|
| `SearchResult` | `path`, `score: f32`, `chunk`, `rank: usize` | `search/mod.rs:35-41` |
| `IndexStats` | `bm25_docs`, `vector_chunks`, `bm25_warm`, `mode` | `search/mod.rs:44-54` |
| Index row | volume-scoped chunk text plus its retrieval structure | per backend |
| `IndexMode` | `none` \| `bm25` \| `rag` \| `both`, stored on the project row | S1, `storage/traits.rs:44-96` |

Column types come from S4 FR-406: `Tsvector` is a real type on PostgreSQL and degrades to `TEXT` on SQLite; `Vector(n)` is real on PostgreSQL with pgvector.

## 9. Impact Analysis

### 9.1 Affected Components

| File/Module | Impact Type | Description |
|---|---|---|
| `crates/mcp-fs/src/search/mod.rs` | Specified, unchanged | FR-703, FR-705..FR-711 |
| `crates/mcp-fs/src/search/{bm25_sqlite,bm25_pg,vector_pg,vector_sqlite}.rs` | Specified, unchanged | FR-706, FR-707 |
| `crates/mcp-fs/src/search/chunker.rs` | Specified, unchanged | FR-712 |
| `crates/mcp-fs/src/search/fusion.rs` | Specified, unchanged | FR-713 |
| `crates/mcp-fs/src/search/rerank.rs` | Specified, unchanged | FR-714 |
| `crates/mcp-fs/src/search/embedding.rs` | Specified, unchanged | FR-715 |
| `crates/mcp-fs/src/search/indexer.rs` | Specified, unchanged | FR-716..FR-721 |
| `crates/mcp-fs/src/tools/search_semantic.rs` | Specified, unchanged | FR-701..FR-705 |

### 9.2 Affected Requirements

S1 FR-030 and FR-031 govern the mode's persistence and vocabulary; FR-721 specifies what a change to it does. S2 FR-201's single-implementation principle is mirrored by FR-716. S4 FR-406 and FR-410 supply the column types and the feature implication. Nothing is modified.

### 9.3 Affected Tests

| Test location | Coverage today | Action |
|---|---|---|
| `crates/mcp-fs/src/search/e2e.rs` (2960 lines) | the subsystem end to end | Keep; annotate with ids |
| `crates/mcp-fs/src/search/chunker.rs` `#[cfg(test)]` | windows and overlap | Keep; annotate |
| `crates/mcp-fs/src/search/fusion.rs` `#[cfg(test)]` | RRF merge and determinism | Keep; annotate |
| `crates/mcp-fs/src/search/indexer.rs` `#[cfg(test)]` | hooks and mode changes | Keep; annotate |
| `tests/functional/scenarios/05_search.sh` | search tools over HTTP | Extend |

`search/e2e.rs` being 2960 lines against roughly 3150 lines of implementation means this subsystem is already the best-tested in the repository. §12 therefore specifies fewer new tests than the other specs and leans on annotation.

### 9.4 Affected Documentation

| Document | Section | Action |
|---|---|---|
| `AGENTS.md` | documentation index | Reference this spec |
| `.agent_docs/search.md` | tools, modes, config, per-backend caveats | Cross-reference §6 |

### 9.5 Dependencies & Risks

1. **External endpoints on the indexing path.** Embedding and reranking are network calls to services outside the deployment. FR-717 and FR-718 contain the blast radius, but a persistently failing endpoint means a silently stale index (TBD-701).
2. **Unbounded detached tasks** (TBD-702).
3. **Vector backends need `rag`**, which implies `postgres`, so a SQLite deployment wanting vectors still compiles the PostgreSQL driver. That is a build-size consequence, not a runtime one.

## 10. Documentation Requirements

### 10.1 README.md
State that search is off by default and that vector search needs a Cargo feature and an external embedding endpoint.

### 10.2 AGENTS.md & .agent_docs/
- `AGENTS.md`: add this spec to the index.
- `.agent_docs/search.md`: cross-reference §6; keep its per-backend caveats, which this spec does not duplicate.

### 10.3 docs/*
None required.

## 11. Traceability Matrix

| Scenario | Functional Req | E2E Tests (Happy) | E2E Tests (Failure) | E2E Tests (Edge) |
|---|---|---|---|---|
| SC-701 | FR-705, FR-721 | E2E-701 | E2E-702, E2E-703 | E2E-704, E2E-705 |
| SC-702 | FR-701, FR-702, FR-704, FR-712, FR-720 | E2E-706, E2E-707 | E2E-708, E2E-709, E2E-710 | E2E-711, E2E-712, E2E-713 |
| SC-703 | FR-703, FR-713, FR-714, FR-715 | E2E-714, E2E-715 | E2E-716, E2E-717, E2E-718 | E2E-719, E2E-720, E2E-721 |
| SC-704 | FR-716, FR-717, FR-718, FR-719 | E2E-722 | E2E-723, E2E-724, E2E-725 | E2E-726, E2E-727, E2E-728 |
| SC-705 | FR-706, FR-707, FR-708, FR-709, FR-710, FR-711 | E2E-729, E2E-730 | E2E-731, E2E-732, E2E-733 | E2E-734, E2E-735, E2E-736 |

Per-FR coverage:

| FR | Tests | FR | Tests |
|---|---|---|---|
| FR-701 | E2E-706, E2E-708, E2E-711 | FR-712 | E2E-707, E2E-710, E2E-712, E2E-713 |
| FR-702 | E2E-706, E2E-709, E2E-711 | FR-713 | E2E-715, E2E-717, E2E-719 |
| FR-703 | E2E-714, E2E-716, E2E-720 | FR-714 | E2E-716, E2E-718, E2E-721 |
| FR-704 | E2E-707, E2E-709, E2E-712 | FR-715 | E2E-714, E2E-718, E2E-720 |
| FR-705 | E2E-701, E2E-702, E2E-704 | FR-716 | E2E-722, E2E-726, E2E-728 |
| FR-706 | E2E-729, E2E-731, E2E-734 | FR-717 | E2E-722, E2E-723, E2E-726 |
| FR-707 | E2E-729, E2E-732, E2E-735 | FR-718 | E2E-723, E2E-724, E2E-727 |
| FR-708 | E2E-731, E2E-733, E2E-736 | FR-719 | E2E-725, E2E-727, E2E-728 |
| FR-709 | E2E-732, E2E-733, E2E-734 | FR-720 | E2E-711, E2E-713, E2E-722 |
| FR-710 | E2E-730, E2E-735, E2E-736 | FR-721 | E2E-701, E2E-703, E2E-705 |
| FR-711 | E2E-730, E2E-734, E2E-735 | | |

## 12. End-to-End Test Suite

**Placement.** Most tests live in `search/e2e.rs`, which already exercises the subsystem end to end, plus the per-file `#[cfg(test)]` modules. Tool-level tests extend `tests/functional/scenarios/05_search.sh`. Vector tests are **opt-in**: they need the `rag` feature and an embedding endpoint, and skip with a message otherwise.

**Fixtures:** project `spec-search` with `ALICE` as member; files `/doc1.md` containing `the quick brown fox jumps`, `/doc2.md` containing `a slow green turtle walks`, `/long.md` of 5000 characters; a stub embedding endpoint returning deterministic vectors so vector tests need no real provider.

### 12.1 Test Summary

| Test ID | Action | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-701 | Existing | Core Journey | SC-701 | FR-705, FR-721 | Critical |
| E2E-702 | Existing | Error | SC-701 | FR-705 | High |
| E2E-703 | Existing | Error | SC-701 | FR-721 | Critical |
| E2E-704 | New | Edge | SC-701 | FR-705, FR-721 | High |
| E2E-705 | New | Edge | SC-701 | FR-721 | Critical |
| E2E-706 | Existing | Core Journey | SC-702 | FR-701, FR-702 | Critical |
| E2E-707 | Existing | Feature | SC-702 | FR-704, FR-712 | Critical |
| E2E-708 | Existing | Security | SC-702 | FR-701 | Critical |
| E2E-709 | New | Error | SC-702 | FR-702, FR-704 | High |
| E2E-710 | Existing | Error | SC-702 | FR-712 | High |
| E2E-711 | New | Edge | SC-702 | FR-701, FR-702, FR-720 | Critical |
| E2E-712 | New | Edge | SC-702 | FR-704, FR-712 | High |
| E2E-713 | Existing | Edge | SC-702 | FR-712, FR-720 | Critical |
| E2E-714 | Existing | Core Journey | SC-703 | FR-703, FR-715 | Critical |
| E2E-715 | Existing | Feature | SC-703 | FR-713 | Critical |
| E2E-716 | New | Error | SC-703 | FR-703, FR-714 | High |
| E2E-717 | Existing | Error | SC-703 | FR-713 | High |
| E2E-718 | New | Error | SC-703 | FR-714, FR-715 | Critical |
| E2E-719 | Existing | Edge | SC-703 | FR-713 | Critical |
| E2E-720 | New | Edge | SC-703 | FR-703, FR-715 | High |
| E2E-721 | New | Edge | SC-703 | FR-714 | Medium |
| E2E-722 | Existing | Core Journey | SC-704 | FR-716, FR-717, FR-720 | Critical |
| E2E-723 | New | Error | SC-704 | FR-717, FR-718 | Critical |
| E2E-724 | New | Error | SC-704 | FR-718 | Critical |
| E2E-725 | Existing | Error | SC-704 | FR-719 | High |
| E2E-726 | New | Edge | SC-704 | FR-716, FR-717 | High |
| E2E-727 | New | Edge | SC-704 | FR-718, FR-719 | High |
| E2E-728 | New | Edge | SC-704 | FR-716, FR-719 | High |
| E2E-729 | Existing | Core Journey | SC-705 | FR-706, FR-707 | Critical |
| E2E-730 | Existing | Feature | SC-705 | FR-710, FR-711 | Critical |
| E2E-731 | New | Error | SC-705 | FR-706, FR-708 | High |
| E2E-732 | Existing | Error | SC-705 | FR-707, FR-709 | Critical |
| E2E-733 | New | Error | SC-705 | FR-708, FR-709 | High |
| E2E-734 | New | Edge | SC-705 | FR-706, FR-709, FR-711 | High |
| E2E-735 | New | Edge | SC-705 | FR-707, FR-710, FR-711 | Critical |
| E2E-736 | New | Edge | SC-705 | FR-708, FR-710 | Medium |

**Coverage Statistics** (36 tests):
- Happy path (Core Journey + Feature): 10
- Failure/error (Error + Security): 15
- Edge cases: 11
- Happy:Failure ratio: 1:1.5

### 12.2 New Test Specifications

#### E2E-704: Status counts chunks rather than files
- **Category:** Edge | **Scenario:** SC-701 | **Requirements:** FR-705, FR-721
- **Preconditions:** project `spec-search` in mode `bm25`; `/long.md` of 5000 characters and `/doc1.md` of 25 characters.
- **Steps:**
  - Given both files indexed with `chunk_size` 1000 and `chunk_overlap` 100
  - When `search.status` is called
  - Then `bm25_docs` is greater than 2, proving it counts chunks and not files
  - And `mode` equals `bm25`
  - And `vector_chunks` equals 0, because RAG is not enabled
  - And `bm25_warm` is true
- **Priority:** High

#### E2E-705: Switching to none wipes immediately and only that volume
- **Category:** Edge | **Scenario:** SC-701 | **Requirements:** FR-721
- **Preconditions:** two projects `spec-search` and `spec-other`, both in mode `bm25`, both with indexed content.
- **Steps:**
  - Given both projects indexed
  - When `admin.set_index_mode` sets `spec-search` to `none`
  - Then the call returns `previous_mode` `bm25` and `index_mode` `none`
  - And `search.status` for `spec-search` reports `bm25_docs` 0 immediately, without polling
  - And `search.status` for `spec-other` still reports its original count, proving the wipe is volume-scoped
  - And `search.query` on `spec-search` returns an empty result list rather than an error
- **Priority:** Critical

#### E2E-709: Deleting a path removes exactly its chunks
- **Category:** Error | **Scenario:** SC-702 | **Requirements:** FR-702, FR-704
- **Preconditions:** `/doc1.md` and `/doc2.md` both indexed.
- **Steps:**
  - Given a recorded `bm25_docs` count of N
  - When `search.delete` is called for `/doc1.md`
  - Then the reported deleted count is the number of chunks `/doc1.md` had
  - And `search.status` reports a lower `bm25_docs`
  - And `search.query` for `quick brown fox` returns no result whose `path` is `/doc1.md`
  - And a query matching `/doc2.md` still returns it
- **Priority:** High

#### E2E-711: Re-indexing a path is idempotent
- **Category:** Edge | **Scenario:** SC-702 | **Requirements:** FR-701, FR-702, FR-720
- **Preconditions:** `/doc1.md` indexed once.
- **Steps:**
  - Given a recorded `bm25_docs` count of N after the first index
  - When `search.index` is called again for the same path with the same parameters
  - Then the reported inserted count equals the first call's
  - And `search.status` still reports exactly N, proving delete-then-insert rather than append
  - And a query returns each chunk once, with no duplicate path and rank pair
- **Priority:** Critical

#### E2E-712: A recursive index and delete cover a subtree
- **Category:** Edge | **Scenario:** SC-702 | **Requirements:** FR-704, FR-712
- **Preconditions:** `/notes/a.md` and `/notes/b.md` exist.
- **Steps:**
  - Given both files
  - When `search.index` is called on `/notes` with `recursive` true
  - Then a query matching either file returns it
  - And when `search.delete` is called on `/notes` with `recursive` true
  - Then queries return neither, and the reported deleted count covers both files' chunks
- **Priority:** High

#### E2E-716: A failing reranker degrades rather than failing the query
- **Category:** Error | **Scenario:** SC-703 | **Requirements:** FR-703, FR-714
- **Preconditions:** reranking enabled and pointed at a closed port; content indexed.
- **Steps:**
  - Given an unreachable rerank endpoint
  - When `search.query` is called with `rerank` true
  - Then the call succeeds
  - And results are returned in their pre-rerank order
  - And the same query with `rerank` false returns the identical ordering
- **Priority:** High

#### E2E-718: Embedding configuration is read from the environment
- **Category:** Error | **Scenario:** SC-703 | **Requirements:** FR-714, FR-715
- **Preconditions:** a stub embedding endpoint requiring an API key; `api_key_env` naming a variable.
- **Steps:**
  - Given the named variable unset
  - When a vector query is attempted
  - Then it fails with an error that does **not** contain the key value
  - And after the variable is set, the same query succeeds
  - And the config file itself contains no key material
- **Priority:** Critical

#### E2E-720: An absent mode argument falls back to the server mode
- **Category:** Edge | **Scenario:** SC-703 | **Requirements:** FR-703, FR-715
- **Preconditions:** a server configured with `search.mode` `bm25`.
- **Steps:**
  - Given content indexed
  - When `search.query` is called with no `mode` argument
  - Then results are returned
  - And they are identical to the same query issued with `mode` explicitly set to `bm25`
  - And requesting `mode` `rag` on this server fails, because no vector backend was built
- **Priority:** High

#### E2E-721: top_k and top_n bound the result and the rerank payload
- **Category:** Edge | **Scenario:** SC-703 | **Requirements:** FR-714
- **Preconditions:** 30 indexed chunks matching a query; reranking enabled with `top_n` 5 against a recording stub.
- **Steps:**
  - Given that setup
  - When `search.query` is called with `top_k` 3
  - Then exactly 3 results are returned
  - And their `rank` values are 1, 2 and 3
  - And the rerank stub recorded a request carrying at most 5 documents
- **Priority:** Medium

#### E2E-723: A write returns before its indexing completes
- **Category:** Error | **Scenario:** SC-704 | **Requirements:** FR-717, FR-718
- **Preconditions:** a project in mode `rag` with a deliberately slow stub embedding endpoint, delaying 2 seconds per call.
- **Steps:**
  - Given that endpoint
  - When `fs.write` is called for a new file
  - Then the write returns in well under 2 seconds
  - And `search.status` immediately afterwards does not yet count the new chunks
  - And after waiting 5 seconds the chunks are counted, proving the work happened in a detached task
- **Priority:** Critical

#### E2E-724: An indexing failure never fails the write
- **Category:** Error | **Scenario:** SC-704 | **Requirements:** FR-718
- **Preconditions:** a project in mode `rag` with an embedding endpoint pointed at a closed port.
- **Steps:**
  - Given an unreachable embedding endpoint
  - When `fs.write` is called for a new file
  - Then the write succeeds and reports its byte count
  - And `fs.read` returns the written content
  - And `search.status` shows the chunks were not added
  - And no error is returned to the caller at any point
- **Priority:** Critical

#### E2E-726: Every write surface indexes identically
- **Category:** Edge | **Scenario:** SC-704 | **Requirements:** FR-716, FR-717
- **Preconditions:** a project in mode `bm25`.
- **Steps:**
  - Given the same text written four ways: `fs.write`, the REST upload, `fs.apply_patch` adding a file, and an S5 document companion
  - When the index settles
  - Then `search.query` finds all four paths
  - And each has the same chunk count for the same text, proving one mapping serves every surface
- **Priority:** High

#### E2E-727: A delete de-indexes through every delete surface
- **Category:** Edge | **Scenario:** SC-704 | **Requirements:** FR-718, FR-719
- **Preconditions:** indexed files at `/x.md` and `/dir/y.md`.
- **Steps:**
  - Given both indexed
  - When `/x.md` is deleted through `fs.delete` and `/dir` is deleted recursively
  - Then queries return neither path once the index settles
  - And a `fs.move` of a third indexed file de-indexes its old path and indexes the new one
- **Priority:** High

#### E2E-728: A project in mode none indexes nothing
- **Category:** Edge | **Scenario:** SC-704 | **Requirements:** FR-716, FR-719
- **Preconditions:** project in mode `none`; search enabled server-wide.
- **Steps:**
  - Given a project whose mode is `none`
  - When files are written through `fs.write` and the REST upload
  - Then `search.status` reports `bm25_docs` 0 throughout
  - And `search.query` returns an empty list
  - And an explicit `search.index` call still indexes the path, because the mode governs automatic indexing rather than the explicit tool
- **Priority:** High

#### E2E-731: A missing feature is refused with a message naming it
- **Category:** Error | **Scenario:** SC-705 | **Requirements:** FR-706, FR-708
- **Preconditions:** a default build with neither `postgres` nor `rag`.
- **Steps:**
  - Given `search.mode` `bm25` and `infra.meta.backend` `postgres`
  - When the backend is built
  - Then it fails with `ERR_NOT_SUPPORTED`
  - And the message names the `postgres` feature
  - And with `infra.meta.backend` `sqlite` the same mode builds the Tantivy backend successfully
- **Priority:** High

#### E2E-733: A vector mode without an endpoint is refused twice over
- **Category:** Error | **Scenario:** SC-705 | **Requirements:** FR-708, FR-709
- **Preconditions:** a build with the `rag` feature.
- **Steps:**
  - Given `search.mode` `both` and an empty `search.embedding.endpoint`
  - When the server boots
  - Then boot validation rejects the configuration
  - And when `build_backend` is called directly, bypassing boot validation, it also fails with `ERR_INVALID_ARGUMENT`
  - And the message names `search.embedding.endpoint`
- **Priority:** High

#### E2E-734: One query cannot reach another project's chunks
- **Category:** Edge | **Scenario:** SC-705 | **Requirements:** FR-706, FR-709, FR-711
- **Preconditions:** `spec-search` and `spec-other` both indexed, with `spec-other` holding a unique term `zzunique`.
- **Steps:**
  - Given both projects indexed in the same backend
  - When `search.query` is called on `spec-search` for `zzunique`
  - Then the result list is empty
  - And the same query on `spec-other` returns its file
  - And a member of `spec-search` who is not a member of `spec-other` receives `ERR_FORBIDDEN` when querying the latter
- **Priority:** High

#### E2E-735: One backend serves every project
- **Category:** Edge | **Scenario:** SC-705 | **Requirements:** FR-707, FR-710, FR-711
- **Preconditions:** two projects with **different** index modes, one `bm25` and one `both`, on a server configured with `search.mode` `both`.
- **Steps:**
  - Given that configuration
  - When both projects are indexed and queried
  - Then both are served by the same process-wide backend
  - And the project in mode `bm25` has `vector_chunks` 0 while the project in mode `both` has a non-zero count
  - And neither project's mode changed which engine answered, only whether vectors were produced
- **Priority:** Critical

#### E2E-736: A default build supports exactly bm25 on SQLite
- **Category:** Edge | **Scenario:** SC-705 | **Requirements:** FR-708, FR-710
- **Preconditions:** a default build.
- **Steps:**
  - Given no optional feature compiled in
  - When `search.mode` is `bm25` with `infra.meta.backend` `sqlite`
  - Then the backend builds and the four tools register
  - And `search.mode` `rag` fails, naming the `rag` feature
  - And `search.mode` `both` fails the same way
- **Priority:** Medium

### 12.3 Modified Test Specifications

None.

### 12.4 Removed Tests

None.

## 13. Consistency Notes

1. **Two senses of "mode".** The project index mode (S1 `IndexMode`) decides whether content is indexed; the server query mode (`search.mode`) decides which engine answers. §4.5 names both; E2E-735 pins the distinction.
2. **"Document" means chunk.** `IndexStats.bm25_docs` counts chunks, not files (FR-705). E2E-704 pins it, because the field name invites the opposite reading.
3. **S1 FR-030 and S7 FR-721 split the mode change.** S1 owns the gate, the persistence and the return shape; S7 owns the index side effects. `reindex_started` is described identically in both.
4. **S5 FR-514 and S7 FR-718 apply the same rule** to two different external dependencies: once bytes are committed, a downstream failure does not retract them.

## 14. Migration & Implementation Notes

No production code change. Test work, in order:

1. **Annotate `search/e2e.rs` first.** At 2960 lines it already covers most of this specification; mapping its existing tests to `E2E-7xx` ids will show which of the "New" tests below are in fact already written under another name. Do this before writing anything new.
2. **Add the pure-function tests** next: chunking boundaries and RRF determinism live in `chunker.rs` and `fusion.rs` and need no server.
3. **Add the SQLite BM25 tests** (E2E-704, E2E-705, E2E-709, E2E-711, E2E-712, E2E-728, E2E-731, E2E-736), which need neither the `rag` feature nor a network endpoint.
4. **Add the timing-sensitive tests** (E2E-723, E2E-724) with a stub embedding endpoint whose delay and failure mode are controlled by the test. E2E-723 asserts a write returns fast and the index settles later, so it needs generous upper bounds to avoid flakiness: assert the write is under 2 seconds and allow 5 seconds for settling.
5. **Add the vector tests last** (E2E-718, E2E-720, E2E-733, E2E-734, E2E-735), gated on the `rag` feature and skipping with a message otherwise. Use the deterministic stub embedding endpoint so results are assertable.
6. **E2E-726 spans four subsystems** (S2, S3, S5, S7). Place it in the functional scenario suite rather than in a unit module, and run it after each of those specs' own tests are green.

## 15. Open Questions & TBDs

- **TBD-701:** Indexing failures are log lines with no counter and no status field (§7.5). A persistently failing embedding endpoint produces a silently stale index. Adding a failure count to `IndexStats` is the obvious fix and does not exist.
- **TBD-702:** Detached indexing tasks are unbounded (§7.7). A burst of writes spawns a burst of tasks with no semaphore.
- **TBD-703:** A full rebuild after a mode change has no throttle and no progress report beyond chunk counts. On a large project it competes with live traffic.
- **TBD-704:** `ASSUMED:` the claim in §9.3 that `search/e2e.rs` covers most of this specification is based on its size, not on a mapping of its tests. §14 step 1 exists to replace this assumption with evidence.

## 16. Glossary

| Term | Definition | Context |
|---|---|---|
| **Chunk** | One indexed window of text, the unit of both storage and retrieval. | Index |
| **Project index mode** | The per-project setting deciding whether content is indexed: `none`, `bm25`, `rag`, `both`. | Auto indexing |
| **Server query mode** | The `search.mode` setting deciding which engine answers a query. | Retrieval |
| **BM25** | The full-text ranking function, served by Tantivy on SQLite or tsvector on PostgreSQL. | Retrieval |
| **Vector search** | Cosine-similarity KNN over embeddings, served by pgvector or sqlite-vec. | Retrieval |
| **Hybrid retrieval** | Querying both engines and merging the two result lists. | Retrieval |
| **RRF** | Reciprocal Rank Fusion with k=60, the deterministic merge used for hybrid retrieval. | Retrieval |
| **Reranking** | An optional external call reordering merged results by relevance score. | Retrieval |
| **Embedding** | The vector produced for a chunk by an external OpenAI-compatible endpoint. | Embedding |
| **Overlap** | The characters a chunk shares with its predecessor, so context survives the cut. | Index |
| **Auto indexing** | The side effects a project's mode attaches to writes and deletes. | Auto indexing |
| **Detached task** | The background task a write hands its indexing to, so the write never waits. | Auto indexing |
| **Wipe** | The synchronous removal of a volume's chunks when its mode becomes `none`. | Auto indexing |
| **Rebuild** | The background full index started when a mode becomes active. | Auto indexing |
| **Warm index** | A Tantivy index whose directory exists and is non-empty. | Index |

## 17. Interview Decisions Log

Produced non-interactively from the code.

- **DEC-701:** The two rules of auto indexing are specified as unwanted-behaviour requirements (FR-717, FR-718) rather than as design notes. **Rationale:** both are trades with a visible consequence, index lag and silent staleness, and a future change that made indexing synchronous "for correctness" would break the write path's latency guarantee without any test objecting. **Alternatives considered:** documenting them in the module comment, which is where they live today. **Implemented by:** FR-717, FR-718, E2E-723, E2E-724. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/search/indexer.rs:6-15`.
- **DEC-702:** The distinction between project index mode and server query mode is elevated to the glossary and pinned by a test. **Rationale:** one word covering two settings is exactly the term overload §4.5 exists to prevent, and the two behave differently. **Alternatives considered:** renaming one of them, which is a code change outside a retro-spec. **Implemented by:** FR-710, §4.5, E2E-735. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/search/indexer.rs:16-18`.
- **DEC-703:** `bm25_docs` counting chunks rather than files is specified explicitly and tested. **Rationale:** the field name reads as a file count, so an integrator will misread it unless the spec is blunt. **Alternatives considered:** leaving it to the field's doc comment. **Implemented by:** FR-705, E2E-704, §13 item 2. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/search/mod.rs:44-47`.
- **DEC-704:** RRF determinism is specified as a requirement, not an implementation detail. **Rationale:** a non-deterministic search result makes every downstream test flaky, and the tie-break by path is what prevents it. **Alternatives considered:** specifying the merge without the ordering guarantee. **Implemented by:** FR-713, E2E-719. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/search/fusion.rs:7-8,14`.
- **DEC-705:** §12 specifies fewer new tests than the other specs, and §14 makes annotation of `search/e2e.rs` the first task. **Rationale:** at 2960 lines against roughly 3150 lines of implementation, this subsystem is already the best tested in the repository; writing new tests before mapping the existing ones would duplicate coverage. **Alternatives considered:** specifying a full complement of new tests regardless. **Implemented by:** §9.3, §14 step 1, TBD-704. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/search/e2e.rs` is 2960 lines; the other nine files total 3154.

## 18. Implementability Audit

Audited as part of the cross-spec audit of S1 through S8 on 2026-09-18; see
`specs/AUDIT.md` for the method, the full findings and the limitations. Sub-agent
execution was unavailable (provider budget error), so the contract's fresh-context
auditor was replaced by mechanical verification plus a targeted reading pass. That
substitution is weaker in one specific respect, recorded in `AUDIT.md`.

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|-------|--------------------------|-------------------|---------|
| 1 | 0 | 0 | IMPLEMENTABLE-WITH-DRIFT |

**Amendments applied:** none required
**Drift registered:** none. Every A finding was evident and amended in place, which
the contract prefers to a register entry.

## 19. Implementation Drift Register

Empty, and deliberately so: all A findings from the cross-spec audit were evident
corrections applied in place rather than drift to resolve during implementation.
See `specs/AUDIT.md` for each finding and its evidence.
