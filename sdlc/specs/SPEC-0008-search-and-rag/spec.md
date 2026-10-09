> Id: SPEC-0008
> Nature: FEAT
> Status: as-built
> Area: search-and-rag
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# mcp-fs Search and RAG — Specification

## 1. Summary

Search over volume content: four `search.*` tools, four backends behind one trait, automatic indexing driven by a per-project mode, and a hybrid retrieval path merging full-text and vector results.

Three rules define the layer. A write never waits for the index: indexing is handed to a detached task, so a slow or absent embedding endpoint costs a write nothing, at the price of the index lagging the volume by one backend call. An index failure is a log line, not an error: the caller already committed bytes, and refusing the write afterwards would be a lie. A project's mode decides whether its content is indexed, not which engine indexes it: the engine is a server-wide configuration choice.

The backend set is chosen by combining `search.mode` with `infra.meta.backend`: Tantivy BM25 on SQLite, PostgreSQL tsvector, pgvector, sqlite-vec. Vector search needs the `rag` Cargo feature, which implies `postgres`.

## 2. Current State

### 2.1 Prior specifications
None superseded by this document.

### 2.2 Deprecated behaviour
None.

### 2.3 As-built architecture
- Trait `SearchBackend` (`crates/core/src/search/mod.rs`), six methods: `index_path`, `delete_path`, `delete_all`, `query_bm25`, `query_vector`, and a statistics call. A combined "both" backend is assembled by the factory rather than by a wrapper type, so the trait stays flat.
- Factory `build_backend` (`crates/core/src/search/mod.rs`), keyed on `(search.mode, infra.meta.backend)`.
- Auto indexing in `crates/core/src/search/indexer.rs`, with `AUTO_CHUNK_SIZE` 1000 and `AUTO_CHUNK_OVERLAP` 100, and hooks `after_write`, `after_write_reread`, `after_companion`, `after_patch`, `after_delete`, `after_delete_many`.
- Chunking in `crates/core/src/search/chunker.rs`: fixed windows with sentence-boundary tolerance.
- Fusion in `crates/core/src/search/fusion.rs`: RRF with `K = 60`.
- Reranking in `crates/core/src/search/rerank.rs`: a Cohere/Jina-compatible HTTP call.
- Tools in `crates/core/src/tools/search_semantic.rs`: the four `search.*` tools.

## 3. Scope

### 3.1 In scope
- The four tools: `search.index`, `search.query`, `search.delete`, `search.status`.
- The `SearchBackend` trait and its four implementations.
- Backend selection from `search.mode` crossed with `infra.meta.backend`, and its feature gating.
- Chunking: window size, overlap, boundary tolerance.
- Auto indexing: the mode-to-operation mapping, the detached-task rule, the failure rule, every write and delete hook.
- Mode-change side effects: the wipe, the background rebuild, and their ordering relative to the stored mode.
- Hybrid retrieval: RRF merge and its determinism.
- Reranking: when it applies and what it calls.
- Embeddings: the OpenAI-compatible endpoint, model, dimension and key handling.
- `IndexStats` and what `search.status` reports.

### 3.2 Out of scope
- `IndexMode` persistence and the `admin.*` tools that manage it. This document specifies what a mode does, not how it is stored.
- The write and delete operations themselves.
- Text extraction; search indexes whatever text it is handed.
- The relational seam and the vector column types.
- Any ranking-quality claim; this is mechanism, not relevance.

## 4. Actors

| Actor | Description |
|---|---|
| **LLM agent** | Queries for content it cannot find by name. Hybrid search makes a volume searchable by meaning rather than by path. |
| **Project owner** | Chooses the project's index mode, and pays the rebuild cost when switching it on. |
| **Operator** | Decides whether search exists at all, which engine serves it, whether an embedding endpoint is available, and whether reranking is configured. |
| **Embedding provider** | An OpenAI-compatible endpoint. External, optional, potentially slow, which is why writes never wait for it. |
| **Reranker** | A Cohere or Jina-compatible endpoint. External and optional. |

A "document" in the Index context is one indexed chunk, not a file: `IndexStats.bm25_docs` counts chunks. A "mode" is overloaded: the **project index mode** (whether content is indexed) is distinct from the **server query mode**, `search.mode` (which engine answers).

## 5. Usage Scenarios

### SC-001: Owner enables indexing for a project
**Actor:** Project owner. **Preconditions:** `search.enabled` true; for a vector mode, an embedding endpoint configured.
**Flow:**
1. Owner calls `admin.set_index_mode` with `bm25`, `rag` or `both`.
2. The new mode is stored before any index work.
3. The indexer reacts: switching to an active mode starts a background full index of existing files; switching to `none` wipes the index immediately.
4. The tool reports `reindex_started`.
5. Owner polls `search.status` to watch the index fill.

**Postconditions:** the project's existing content is being indexed; new writes are indexed automatically from now on.
**Exceptions:**
- EXC-001a: `search.enabled` false → `ERR_NOT_SUPPORTED` naming the config key
- EXC-001b: a vector mode with no embedding endpoint → `ERR_INVALID_ARGUMENT` at the gate
- EXC-001c: a crash between the stored mode and the wipe → the mode is recorded and the index stale, which one more `set_index_mode` repairs
- EXC-001d: `reindex_started` true means a background pass is running, not that the volume is searchable

### SC-002: Agent indexes a path explicitly
**Actor:** LLM agent. **Preconditions:** member; search enabled.
**Flow:**
1. Agent calls `search.index` with a path, optionally `recursive`, `chunk_size` (default 1000) and `chunk_overlap` (default 100).
2. The backend deletes any existing chunks for that path, then splits the text into overlapping windows and inserts them.
3. The tool reports how many chunks were inserted.

**Postconditions:** the path is searchable; re-indexing it is idempotent because the delete precedes the insert.
**Exceptions:**
- EXC-002a: a directory without `recursive` → only that path is considered
- EXC-002b: a binary or unreadable file → skipped rather than failing the call
- EXC-002c: an empty file → zero chunks, not an error

### SC-003: Agent searches and gets hybrid results
**Actor:** LLM agent. **Preconditions:** member; content indexed.
**Flow:**
1. Agent calls `search.query` with a query, optionally `mode`, `top_k` (default 10) and `rerank` (default true).
2. The mode defaults to the server's configured mode when the caller does not name one.
3. In `bm25` the full-text backend answers; in `rag` vector search answers; in `both` hybrid retrieval queries each and merges the two lists by RRF.
4. When reranking is configured and requested, the merged list is sent to the rerank endpoint and reordered.
5. Results carry `path`, `score`, `chunk` and `rank`.

**Postconditions:** the agent holds ranked chunks with the paths they came from.
**Exceptions:**
- EXC-003a: a vector mode with no embedding endpoint → refused
- EXC-003b: the rerank endpoint unreachable → results are returned unreranked rather than the query failing
- EXC-003c: nothing indexed → an empty result list, not an error
- EXC-003d: a document present in both result lists → ranked above one present in only one

### SC-004: Content is indexed automatically as it changes
**Actor:** LLM agent, implicitly. **Preconditions:** the project's mode is active.
**Flow:**
1. Agent writes a file through any surface: an `fs.*` tool, a REST upload, a patch, or a document companion.
2. The matching hook hands indexing to a detached task and returns immediately.
3. The write's response is returned without waiting.
4. The chunks appear in the index shortly afterwards.
5. A delete removes the path's chunks the same way.

**Postconditions:** the index converges on the volume's content without any caller waiting for it.
**Exceptions:**
- EXC-004a: the embedding endpoint slow or absent → the write still succeeds at full speed
- EXC-004b: indexing fails → a log line, never an error to the caller, because the bytes are already committed
- EXC-004c: a caller that writes and immediately queries → may not see the new content, because the index lags by one backend call
- EXC-004d: the project's mode is `none` → no hook does anything

### SC-005: Operator picks an engine combination
**Actor:** Operator. **Preconditions:** a build whose features match the intended mode.
**Flow:**
1. Operator sets `search.mode` and `infra.meta.backend`.
2. The factory selects the backend pair from that combination.
3. A vector mode additionally requires an embedding endpoint, checked both at boot validation and at construction.
4. The server starts with a process-wide backend serving every project.

**Postconditions:** every project with an active mode is indexed by that engine.
**Exceptions:**
- EXC-005a: `bm25` with `postgres` and the `postgres` feature absent → `ERR_NOT_SUPPORTED` naming the feature
- EXC-005b: `rag` or `both` with an empty embedding endpoint → `ERR_INVALID_ARGUMENT`
- EXC-005c: `search.enabled` false → no backend is built and the four tools are not registered
- EXC-005d: an unknown mode string → rejected at boot validation

## 6. Functional Requirements

### Tools

#### FR-001 [EARS-U]: Four tools, registered only when search is enabled
> The server SHALL register `search.index`, `search.query`, `search.delete` and `search.status` when `search.enabled` is true, and SHALL register none of them otherwise.
- **Inputs:** `search.enabled`.
- **Outputs:** four tools, or none.
- **Business Rules:** every tool takes `mount_id` and authorizes through the membership gate before any storage access, exactly like the `fs.*` family.
- **Priority:** Must-have

#### FR-002 [EARS-E]: Explicit indexing
> WHEN `search.index` is called THE backend SHALL delete any existing chunks for the path, split its text into overlapping windows, insert them, and report the number inserted.
- **Inputs:** `path`, `recursive` default false, `chunk_size` default 1000, `chunk_overlap` default 100.
- **Outputs:** the inserted chunk count.
- **Business Rules:** delete-then-insert is what makes indexing idempotent, so re-indexing a path never duplicates chunks.
- **Priority:** Must-have

#### FR-003 [EARS-E]: Querying
> WHEN `search.query` is called THE server SHALL answer with ranked results carrying `path`, `score`, `chunk` and `rank`.
- **Inputs:** `query`, `mode` defaulting to the server's configured mode, `top_k` default 10, `rerank` default true.
- **Outputs:** `SearchResult` entries.
- **Business Rules:** an absent `mode` argument falls back to `search.mode` from config, so a caller need not know the deployment's engine.
- **Priority:** Must-have

#### FR-004 [EARS-E]: Deletion from the index
> WHEN `search.delete` is called THE backend SHALL remove every chunk for the path and report the count removed.
- **Inputs:** `path`, `recursive` default false.
- **Outputs:** the deleted chunk count.
- **Priority:** Must-have

#### FR-005 [EARS-E]: Status reporting
> WHEN `search.status` is called THE server SHALL report the BM25 chunk count, the vector chunk count, whether the BM25 index is warm, and the active mode.
- **Inputs:** `mount_id`.
- **Outputs:** `IndexStats`.
- **Business Rules:** `bm25_docs` counts chunks, not files. `vector_chunks` is 0 when RAG is not enabled. `bm25_warm` reports a warm index, meaning the Tantivy index directory exists and is non-empty. This is the tool an owner polls to watch a rebuild progress.
- **Priority:** Must-have

### The backend seam

#### FR-006 [EARS-U]: One trait, four implementations
> Every search engine SHALL implement `SearchBackend`, and the trait SHALL remain flat rather than gaining a combining wrapper type.
- **Inputs:** any search operation.
- **Outputs:** engine-independent results.
- **Business Rules:** the six operations are `index_path`, `delete_path`, `delete_all`, `query_bm25`, the vector query and the statistics call. A combined `both` backend is assembled by the factory, not by a wrapper, which is what keeps the trait from growing a mode parameter.
- **Priority:** Must-have

#### FR-007 [EARS-E]: Backend selection
> WHEN the backend is built THE factory SHALL select an implementation from the pair `(search.mode, infra.meta.backend)`.
- **Inputs:** the two config values.
- **Outputs:** the backend, or `None` when search is disabled.
- **Business Rules:** `bm25` with `postgres` selects the tsvector backend; `bm25` with anything else selects Tantivy; `rag` with `postgres` selects pgvector; `rag` on SQLite selects sqlite-vec. The Tantivy backend is always compiled; the others are feature-gated.
- **Priority:** Must-have

#### FR-008 [EARS-O]: Feature gating
> IF a selected backend's Cargo feature was not compiled in THEN the factory SHALL fail with `ERR_NOT_SUPPORTED` naming the required feature.
- **Inputs:** the selection and the compiled feature set.
- **Outputs:** the refusal, for example `search.mode=bm25 with backend=postgres requires the postgres feature`.
- **Business Rules:** vector backends need `rag`, which implies `postgres`. A default build supports `bm25` on SQLite and nothing else.
- **Priority:** Must-have

#### FR-009 [EARS-O]: Embedding endpoint is required for vector modes
> IF `search.mode` is `rag` or `both` and `search.embedding.endpoint` is empty THEN the factory SHALL fail with `ERR_INVALID_ARGUMENT`.
- **Inputs:** the mode and the endpoint.
- **Outputs:** the refusal.
- **Business Rules:** the check is duplicated deliberately: boot validation performs it, and the factory performs it again so the error is clear when called from a test that skipped boot validation. A vector mode without an endpoint indexes nothing and reports success, which is why the same rule also gates `admin.set_index_mode`.
- **Priority:** Must-have

#### FR-010 [EARS-U]: The backend is process-wide
> The server SHALL build one search backend for the whole process, serving every project.
- **Inputs:** the server config.
- **Outputs:** the shared backend.
- **Business Rules:** a project's index mode decides whether its content is indexed, never which engine indexes it. Per-project engine choice does not exist.
- **Priority:** Must-have

#### FR-011 [EARS-U]: Index rows are volume-scoped
> Every index operation SHALL be scoped by `volume_id`.
- **Inputs:** the volume id.
- **Outputs:** rows belonging to one volume.
- **Business Rules:** `delete_all` is the wipe half of a mode change, so it must be scoped to the one volume: another project's chunks live in the same tables.
- **Priority:** Must-have

### Chunking and retrieval

#### FR-012 [EARS-E]: Chunking with boundary tolerance
> WHEN text is chunked THE chunker SHALL produce windows of at most `size` characters overlapping by `overlap` characters, looking back up to 50 characters for a sentence boundary when the cut falls inside a word.
- **Inputs:** the text, `size`, `overlap`.
- **Outputs:** the chunk list.
- **Business Rules:** boundary characters are `.`, newline and space. `overlap` is clamped below `size`, because an overlap equal to the size loops forever. Empty text or a zero size yields no chunks. The overlap exists so an embedding captures cross-boundary context.
- **Priority:** Must-have

#### FR-013 [EARS-E]: Reciprocal rank fusion
> WHEN both result lists are available THE server SHALL merge them by RRF with k equal to 60, re-ranking from 1.
- **Inputs:** the BM25 and vector lists.
- **Outputs:** one merged list.
- **Business Rules:** RRF with k=60 is parameter-insensitive over a wide range of corpus sizes and needs no calibration. A document appearing in both lists scores above one appearing in only one. The merge is deterministic: ties are broken by path lexicographically, so the output is stable across calls.
- **Priority:** Must-have

#### FR-014 [EARS-O]: Reranking
> IF reranking is enabled, configured and requested THEN the server SHALL send the merged results to the rerank endpoint and return them in relevance order.
- **Inputs:** `rerank` default true; `search.reranking` config with `endpoint`, `model`, `api_key_env` and `top_n`.
- **Outputs:** the reordered list.
- **Business Rules:** the request is `{"model","query","documents"}` and the response `{"results":[{"index","relevance_score"}]}`, a Cohere/Jina-compatible shape. `top_n` bounds how many results are sent. API keys are read from the environment variable named by `api_key_env`, never from the config file.
- **Priority:** Should-have

#### FR-015 [EARS-E]: Embedding calls
> WHEN vectors are needed THE server SHALL call the configured OpenAI-compatible `/v1/embeddings` endpoint with the configured model.
- **Inputs:** `endpoint`, `model`, `api_key_env`, `dimensions`.
- **Outputs:** the vectors.
- **Business Rules:** `dimensions` must match the model's output, and it is the column width the vector type is declared with. An empty `api_key_env` means an unauthenticated endpoint, which is what a local embedding server needs.
- **Priority:** Must-have

### Auto indexing

#### FR-016 [EARS-U]: One place maps a mode to index operations
> The mapping from a project's index mode to index operations SHALL exist in exactly one module, used by every write path.
- **Inputs:** the project mode and the operation.
- **Outputs:** the index side effect.
- **Business Rules:** the write tools, the delete tool, the REST upload and `admin.set_index_mode` all go through it, so every surface behaves the same.
- **Priority:** Must-have

#### FR-017 [EARS-UB]: A write never waits for the index
> The server SHALL NOT block a write or a delete on indexing.
- **Inputs:** any write or delete.
- **Outputs:** the response, returned before indexing completes.
- **Business Rules:** the hook hands the work to a detached task and returns, so a slow or absent embedding endpoint costs a write nothing. The consequence is stated rather than hidden: the index lags the volume by the duration of one backend call, visible to a caller that writes and queries back to back.
- **Priority:** Must-have

#### FR-018 [EARS-UB]: An index failure never fails the operation
> The server SHALL NOT turn an indexing failure into an error returned to the caller.
- **Inputs:** a failing index operation.
- **Outputs:** a log line.
- **Business Rules:** the caller already committed bytes; refusing the write afterwards is a lie. This mirrors the document-service rule for the same reason.
- **Priority:** Must-have

#### FR-019 [EARS-E]: Every mutation path has a hook
> WHEN content is written, patched, companion-generated or deleted THE corresponding hook SHALL index or de-index it.
- **Inputs:** the mutation.
- **Outputs:** the index side effect.
- **Business Rules:** the hooks are `after_write`, `after_write_reread` (for a write whose text the caller does not hold), `after_companion` (for generated Markdown), `after_patch` (walking a patch report), `after_delete` and `after_delete_many`. A move is handled by de-indexing the displaced paths.
- **Priority:** Must-have

#### FR-020 [EARS-U]: Auto-indexing chunk window
> Auto indexing SHALL use a chunk size of 1000 characters and an overlap of 100.
- **Inputs:** the indexed text.
- **Outputs:** the chunks.
- **Business Rules:** the constants match the `search.index` tool's defaults, so explicit and automatic indexing produce the same chunking.
- **Priority:** Must-have

#### FR-021 [EARS-E]: Mode change side effects
> WHEN a project's index mode changes THE indexer SHALL wipe the existing index when moving to `none`, and SHALL start a background full index when moving to an active mode.
- **Inputs:** the old and new modes.
- **Outputs:** the wipe, or a started rebuild reported as `reindex_started`.
- **Business Rules:** the wipe finishes before the tool answers; the rebuild does not. `reindex_started: true` therefore means a background pass is running, not that the volume is searchable. The mode is stored before either happens, so the recoverable failure is a stale index rather than a wiped one nothing rebuilds.
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
- Writes are never slowed by indexing (FR-017); the cost is moved off the request path entirely.
- `top_k` bounds result size; `top_n` bounds how much is sent to the reranker (FR-014).
- Chunk size and overlap bound embedding call volume: smaller chunks mean more vectors and more cost.
- A full rebuild after a mode change is a background pass whose duration is unbounded and unreported beyond `search.status` counts.

### 7.2 Security
- Every tool passes the membership gate before any storage access (FR-001).
- Index rows are volume-scoped, so one project's query cannot return another's chunks (FR-011).
- Embedding and rerank API keys are read from environment variables named in config, never stored in the config file (FR-014, FR-015).
- Indexed chunks are stored server-side and are returned only to members of the owning project.

### 7.3 Usability
- `mode` defaults to the server's configuration, so a caller need not know the deployment (FR-003).
- A vector mode without an endpoint is refused at the gate rather than silently indexing nothing (FR-009).
- `search.status` gives an owner a way to watch a rebuild (FR-005).
- A failing reranker degrades to unreranked results rather than failing the query (EXC-003b).

### 7.4 Reliability
- Indexing is idempotent by delete-then-insert (FR-002).
- The mode is stored before the index is touched, so the failure mode is repairable by repeating the call (FR-021).
- An index failure never corrupts or rejects a committed write (FR-018).
- RRF is deterministic, so identical inputs give identical output (FR-013).

### 7.5 Observability
Because indexing failures are log lines (FR-018) and nothing counts them, a persistently failing embedding endpoint produces a silently stale index and no signal other than log volume. `search.status` shows chunk counts but nothing about failures. Recorded as a finding below.

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
- Detached indexing tasks are unbounded in number: a burst of writes spawns a burst of tasks. Recorded as a finding below.

## 8. E2E Tests

**Placement.** Most tests live in `crates/core/src/search/e2e.rs`, plus per-file `#[cfg(test)]` modules. Tool-level tests extend `tests/functional/scenarios/05_search.sh`. Vector tests are opt-in: they need the `rag` feature and an embedding endpoint, and skip with a message otherwise.

**Fixtures:** project `spec-search` with `ALICE` as member; files `/doc1.md` containing "the quick brown fox jumps", `/doc2.md` containing "a slow green turtle walks", `/long.md` of 5000 characters; a stub embedding endpoint returning deterministic vectors so vector tests need no real provider.

| Test ID | Scenario | FR refs | Category | Priority |
|---|---|---|---|---|
| E2E-001 | SC-001 | FR-005, FR-021 | Core Journey | Critical |
| E2E-002 | SC-001 | FR-005 | Error | High |
| E2E-003 | SC-001 | FR-021 | Error | Critical |
| E2E-004 | SC-001 | FR-005, FR-021 | Edge | High |
| E2E-005 | SC-001 | FR-021 | Edge | Critical |
| E2E-006 | SC-002 | FR-001, FR-002 | Core Journey | Critical |
| E2E-007 | SC-002 | FR-004, FR-012 | Feature | Critical |
| E2E-008 | SC-002 | FR-001 | Security | Critical |
| E2E-009 | SC-002 | FR-002, FR-004 | Error | High |
| E2E-010 | SC-002 | FR-012 | Error | High |
| E2E-011 | SC-002 | FR-001, FR-002, FR-020 | Edge | Critical |
| E2E-012 | SC-002 | FR-004, FR-012 | Edge | High |
| E2E-013 | SC-002 | FR-012, FR-020 | Edge | Critical |
| E2E-014 | SC-003 | FR-003, FR-015 | Core Journey | Critical |
| E2E-015 | SC-003 | FR-013 | Feature | Critical |
| E2E-016 | SC-003 | FR-003, FR-014 | Error | High |
| E2E-017 | SC-003 | FR-013 | Error | High |
| E2E-018 | SC-003 | FR-014, FR-015 | Error | Critical |
| E2E-019 | SC-003 | FR-013 | Edge | Critical |
| E2E-020 | SC-003 | FR-003, FR-015 | Edge | High |
| E2E-021 | SC-003 | FR-014 | Edge | Medium |
| E2E-022 | SC-004 | FR-016, FR-017, FR-020 | Core Journey | Critical |
| E2E-023 | SC-004 | FR-017, FR-018 | Error | Critical |
| E2E-024 | SC-004 | FR-018 | Error | Critical |
| E2E-025 | SC-004 | FR-019 | Error | High |
| E2E-026 | SC-004 | FR-016, FR-017 | Edge | High |
| E2E-027 | SC-004 | FR-018, FR-019 | Edge | High |
| E2E-028 | SC-004 | FR-016, FR-019 | Edge | High |
| E2E-029 | SC-005 | FR-006, FR-007 | Core Journey | Critical |
| E2E-030 | SC-005 | FR-010, FR-011 | Feature | Critical |
| E2E-031 | SC-005 | FR-006, FR-008 | Error | High |
| E2E-032 | SC-005 | FR-007, FR-009 | Error | Critical |
| E2E-033 | SC-005 | FR-008, FR-009 | Error | High |
| E2E-034 | SC-005 | FR-006, FR-009, FR-011 | Edge | High |
| E2E-035 | SC-005 | FR-007, FR-010, FR-011 | Edge | Critical |
| E2E-036 | SC-005 | FR-008, FR-010 | Edge | Medium |

**Coverage:** 36 tests; happy 10, failure 15, edge 11 (happy:failure ≈ 1:1.5).

### Selected test narratives

**E2E-004** (status counts chunks, not files): project in mode `bm25`; `/long.md` (5000 chars) and `/doc1.md` (25 chars) both indexed with `chunk_size` 1000, `chunk_overlap` 100. `search.status` reports `bm25_docs` > 2 (proving chunks not files), `mode` = `bm25`, `vector_chunks` = 0, `bm25_warm` = true.

**E2E-005** (mode-to-none wipes immediately, scoped): two projects both `bm25` with indexed content. Setting one to `none` returns `previous_mode: bm25`, `index_mode: none`; its `search.status` reports `bm25_docs` 0 immediately; the other project's count is unaffected; a query on the wiped project returns an empty list, not an error.

**E2E-011** (re-indexing is idempotent): index `/doc1.md` once, record count N; index again with the same parameters; inserted count equals N; `search.status` still reports N; a query returns each chunk once with no duplicate path/rank pair.

**E2E-018** (embedding key read from the environment): a stub endpoint requiring an API key named by `api_key_env`. With the variable unset, a vector query fails with an error that does not contain the key value; after setting it, the same query succeeds; the config file itself contains no key material.

**E2E-023** (write returns before indexing completes): project in `rag` mode with a stub embedding endpoint delaying 2s per call. `fs.write` for a new file returns well under 2s; `search.status` immediately after does not yet count the new chunks; after 5s the chunks are counted.

**E2E-024** (indexing failure never fails the write): embedding endpoint pointed at a closed port. `fs.write` succeeds and reports its byte count; `fs.read` returns the content; `search.status` shows the chunks were never added; no error reaches the caller at any point.

**E2E-033** (vector mode without endpoint refused twice): `search.mode` `both`, empty `search.embedding.endpoint`. Boot validation rejects it; calling `build_backend` directly, bypassing boot validation, also fails with `ERR_INVALID_ARGUMENT` naming `search.embedding.endpoint`.

**E2E-035** (one backend serves every project): two projects, one `bm25` and one `both`, server configured `search.mode` `both`. Both served by the same process-wide backend; the `bm25` project has `vector_chunks` 0, the `both` project has a non-zero count; neither project's mode changed which engine answered.

**E2E-036** (default build supports exactly bm25 on SQLite): no optional feature compiled in. `bm25` + `sqlite` builds and registers the four tools; `rag` fails naming the `rag` feature; `both` fails the same way.

## 9. Glossary

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

## 10. Confidence Notes

- Spot-checked against the current tree: `crates/core/src/search/{mod.rs,indexer.rs,fusion.rs,chunker.rs}` confirmed for the trait shape, `SearchResult`/`IndexStats` fields, `AUTO_CHUNK_SIZE`/`AUTO_CHUNK_OVERLAP` = 1000/100, the six hook functions (`after_write`, `after_write_reread`, `after_companion`, `after_patch`, `after_delete`, `after_delete_many`), and RRF's `K = 60` constant with deterministic path tie-breaking.
- **Path drift found and corrected:** the source spec cites `crates/mcp-fs/src/search/*.rs` throughout; the code actually lives at `crates/core/src/search/*.rs` (per `AGENTS.md`'s documented crate layout: `crates/core` holds all logic, `crates/mcp-fs` is a 9-line `main.rs`). This spec and the companion design.md use the corrected path. See design.md §9 for the mapping.
- Content, business rules and behaviour claims in the source spec were not found to be materially wrong anywhere spot-checked; only the path prefix was stale.
- Confidence: high.
