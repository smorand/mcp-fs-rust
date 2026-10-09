> Id: SPEC-0008
> Nature: FEAT
> Status: as-built
> Area: search-and-rag
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# mcp-fs Search and RAG — Design

## 1. Components

| Component | Path | Role |
|---|---|---|
| `SearchBackend` trait | `crates/core/src/search/mod.rs` | the flat interface every engine implements: `index_path`, `delete_path`, `delete_all`, `query_bm25`, `query_vector`, status |
| Factory `build_backend` | `crates/core/src/search/mod.rs` | selects an implementation from `(search.mode, infra.meta.backend)`, feature-gated, assembles the `both` combination |
| Tantivy BM25 backend | `crates/core/src/search/bm25_sqlite.rs` | always compiled; the default-build backend |
| PostgreSQL tsvector backend | `crates/core/src/search/bm25_pg.rs` | `postgres` feature |
| pgvector backend | `crates/core/src/search/vector_pg.rs` | `rag` feature (implies `postgres`) |
| sqlite-vec backend | `crates/core/src/search/vector_sqlite.rs` | `rag` feature |
| Indexer / auto-indexing hooks | `crates/core/src/search/indexer.rs` | maps project mode to index operations; detached-task dispatch; mode-change wipe/rebuild |
| Chunker | `crates/core/src/search/chunker.rs` | fixed windows, sentence-boundary tolerance, overlap clamp |
| Fusion | `crates/core/src/search/fusion.rs` | RRF merge, `K = 60`, deterministic path tie-break |
| Reranker | `crates/core/src/search/rerank.rs` | Cohere/Jina-compatible HTTP reorder, `top_n` bound |
| Embedding client | `crates/core/src/search/embedding.rs` | OpenAI-compatible `/v1/embeddings` call |
| Tools | `crates/core/src/tools/search_semantic.rs` | `search.index`, `search.query`, `search.delete`, `search.status` |
| Test suite | `crates/core/src/search/e2e.rs` (2960 lines) | subsystem end-to-end, roughly half the subsystem's total line count |

## 2. Flows

### 2.1 Explicit index (`search.index`)
1. Tool authorizes membership on `mount_id`.
2. `backend.delete_path(volume_id, path)` removes existing chunks.
3. Text is split via `chunker` into windows of `chunk_size` overlapping by `chunk_overlap`.
4. `backend.index_path` inserts chunks.
5. Inserted count returned.

### 2.2 Query (`search.query`)
1. Tool authorizes membership.
2. `mode` resolves to the argument or, absent, `search.mode` from config.
3. `bm25` → `query_bm25` only; `rag` → `query_vector` only; `both` → both calls, merged via `fusion::rrf_merge`.
4. If `rerank` is true and reranking is configured, merged results go to `rerank.rs`'s HTTP call; on failure, results are returned unreranked (never an error).
5. Results mapped to `SearchResult { path, score, chunk, rank }`.

### 2.3 Auto indexing on write/delete
1. A write surface (`fs.write`, REST upload, `fs.apply_patch`, document companion) completes its primary operation and returns.
2. The matching hook (`after_write`, `after_write_reread`, `after_companion`, `after_patch`, `after_delete`, `after_delete_many`) is invoked.
3. The hook checks the project's `IndexMode`; if `none`, no-op.
4. Otherwise it spawns a detached task calling `backend.index_path` / `delete_path` with `AUTO_CHUNK_SIZE`/`AUTO_CHUNK_OVERLAP` (1000/100).
5. Any error from the detached task is logged, never propagated.

### 2.4 Mode change (`admin.set_index_mode`)
1. New mode is persisted first (S1-owned).
2. If new mode is `none`: `delete_all(volume_id)` runs synchronously before the tool returns (wipe).
3. If new mode is active (`bm25`/`rag`/`both`): a background task walks every file and calls `index_path`; the tool returns `reindex_started: true` without waiting for it.

## 3. Interfaces

### 3.1 MCP tools
- `search.index(mount_id, path, recursive=false, chunk_size=1000, chunk_overlap=100)` → `{ inserted: usize }`
- `search.query(mount_id, query, mode?, top_k=10, rerank=true)` → `{ results: Vec<SearchResult> }`
- `search.delete(mount_id, path, recursive=false)` → `{ deleted: usize }`
- `search.status(mount_id)` → `IndexStats`

### 3.2 `SearchBackend` trait (6 methods)
async fn index_path(&self, volume_id, path, text, chunk_size, chunk_overlap) -> Result<usize>;
async fn delete_path(&self, volume_id, path) -> Result<usize>;
async fn delete_all(&self, volume_id) -> Result<usize>;
async fn query_bm25(&self, volume_id, query, top_k) -> Result<Vec<SearchResult>>;
async fn query_vector(&self, volume_id, query, top_k) -> Result<Vec<SearchResult>>;
// + a statistics method backing IndexStats

### 3.3 External HTTP contracts
- Embedding: `POST {endpoint}/v1/embeddings`, OpenAI-compatible, model + input → vectors of `dimensions` width.
- Reranking: request `{"model","query","documents"}`, response `{"results":[{"index","relevance_score"}]}`, Cohere/Jina-compatible.

## 4. Data and State

| Structure | Fields | Home |
|---|---|---|
| `SearchResult` | `path: String`, `score: f32`, `chunk: String`, `rank: usize` | `crates/core/src/search/mod.rs` |
| `IndexStats` | `bm25_docs: usize`, `vector_chunks: usize`, `bm25_warm: bool`, `mode: String` | `crates/core/src/search/mod.rs` |
| Index row | volume-scoped chunk text plus its retrieval structure | per backend |
| `IndexMode` | `none` \| `bm25` \| `rag` \| `both`, stored on the project row | S1, `storage/traits.rs` |

Column types: `Tsvector` is a real PostgreSQL type, degrading to `TEXT` on SQLite; `Vector(n)` is real on PostgreSQL with pgvector (S4-owned types, referenced not redefined here).

## 5. Configuration

| Key | Default | Notes |
|---|---|---|
| `search.enabled` | false | gates tool registration and backend construction |
| `search.mode` | — | `bm25` \| `rag` \| `both` |
| `infra.meta.backend` | — | crossed with `search.mode` to select the implementation |
| `search.tantivy_dir` | — | on-disk BM25 index path for the SQLite/Tantivy backend |
| `search.embedding.endpoint` | empty | required non-empty for `rag`/`both` |
| `search.embedding.model` | — | passed to `/v1/embeddings` |
| `search.embedding.dimensions` | — | must match model output; vector column width |
| `search.embedding.api_key_env` | empty | env var name; empty means unauthenticated |
| `search.reranking.*` | off | `endpoint`, `model`, `api_key_env`, `top_n` |

Feature gates: `postgres` (tsvector, pgvector transport), `rag` (implies `postgres`; pgvector + sqlite-vec). Default build: `bm25` + SQLite(Tantivy) only.

## 6. Observability

- Indexing failures: `tracing` log line only, no counter, no `IndexStats` field (gap, see §9 findings).
- No metric/log for detached-task queue depth or concurrency (gap, see §9 findings).
- `search.status` exposes `bm25_docs`, `vector_chunks`, `bm25_warm`, `mode` — the only operator-visible index health signal.

## 7. Decisions Log

- **DEC-001:** The two auto-indexing rules (never block the write; never fail it) are specified as explicit unwanted-behaviour requirements (FR-017, FR-018) rather than left as module-comment design notes. Rationale: both are trades with a visible consequence (index lag, silent staleness); leaving them undocumented risks a future "fix" making indexing synchronous and silently breaking the write-latency guarantee with no test to catch it. Code: `crates/core/src/search/indexer.rs` module doc + `after_write`/`after_delete` dispatch.
- **DEC-002:** Project index mode and server query mode are kept as two distinct glossary terms and pinned by a dedicated test (E2E-035), rather than merged or renamed. Rationale: one word covering two settings is exactly the overload the bounded-context section exists to prevent, and the two behave differently (per-project vs process-wide). Code: `crates/core/src/search/indexer.rs`.
- **DEC-003:** `bm25_docs` counting chunks rather than files is specified explicitly rather than left to the field's doc comment. Rationale: the field name reads as a file count; an integrator will misread it unless stated bluntly. Code: `crates/core/src/search/mod.rs` (`IndexStats`).
- **DEC-004:** RRF determinism (path tie-break) is a requirement, not an implementation detail. Rationale: a non-deterministic search result makes every downstream test flaky. Code: `crates/core/src/search/fusion.rs`.
- **DEC-005:** The test plan leans on annotating the existing 2960-line `e2e.rs` before writing new tests, rather than specifying a full complement of new tests regardless. Rationale: at roughly half the subsystem's total line count, this is already the best-tested area in the repo; writing blind would duplicate coverage.

## 8. Requirement to Code Map

| FR | Code |
|---|---|
| FR-001 | `crates/core/src/tools/all.rs` (conditional registration), `crates/core/src/tools/search_semantic.rs` |
| FR-002 | `crates/core/src/search/mod.rs` (`index_path` contract), `tools/search_semantic.rs` |
| FR-003 | `tools/search_semantic.rs` (mode fallback to config) |
| FR-004 | `search/mod.rs` (`delete_path`) |
| FR-005 | `search/mod.rs` (`IndexStats`) |
| FR-006 | `search/mod.rs` (`SearchBackend` trait) |
| FR-007 | `search/mod.rs` (`build_backend` factory) |
| FR-008 | `search/mod.rs` (feature-gate checks in factory) |
| FR-009 | `search/mod.rs` (embedding-endpoint guard) |
| FR-010 | `search/indexer.rs` (process-wide backend) |
| FR-011 | `search/mod.rs` (`volume_id` scoping on every op) |
| FR-012 | `search/chunker.rs` |
| FR-013 | `search/fusion.rs` (`rrf_merge`, `K = 60`) |
| FR-014 | `search/rerank.rs` |
| FR-015 | `search/embedding.rs`, `config.rs` |
| FR-016 | `search/indexer.rs` (single mapping module) |
| FR-017 | `search/indexer.rs` (detached task dispatch) |
| FR-018 | `search/indexer.rs` (log-only failure) |
| FR-019 | `search/indexer.rs` (`after_write`, `after_write_reread`, `after_companion`, `after_patch`, `after_delete`, `after_delete_many`) |
| FR-020 | `search/indexer.rs` (`AUTO_CHUNK_SIZE`, `AUTO_CHUNK_OVERLAP`) |
| FR-021 | `search/indexer.rs` (mode-change wipe/rebuild) |

## 9. Legacy Mapping

Source: specs/SPEC-0008_2026-09-18_20-10-00-search-and-rag/spec.md (pre-move)

| Resolved | Mapping |
|---|---|
| All `crates/mcp-fs/src/search/*.rs` and `crates/mcp-fs/src/tools/search_semantic.rs` citations | → `crates/core/src/search/*.rs` and `crates/core/src/tools/search_semantic.rs` respectively. The legacy spec was written against a pre-restructure path; `AGENTS.md` documents the current layout (`crates/core` = library with all logic, `crates/mcp-fs` = 9-line `main.rs`). Verified: `indexer.rs`, `mod.rs`, `fusion.rs`, `chunker.rs` all present and matching at the `crates/core/src/search/` path, with cited line-level content (trait shape, `AUTO_CHUNK_SIZE`/`_OVERLAP` = 1000/100, hook names, RRF `K = 60`) confirmed. |
| FR-7xx, SC-7xx, E2E-7xx, DEC-7xx numbering | → renumbered FR-001.., SC-001.., E2E-001.., DEC-001.. in this split, preserving order and content 1:1. |
| TBD-701 (no failure counter in `IndexStats`) | unresolved, see findings below |
| TBD-702 (unbounded detached tasks) | unresolved, see findings below |
| TBD-703 (rebuild has no throttle/progress) | unresolved, see findings below |
| TBD-704 (ASSUMED: e2e.rs coverage claim based on size, not mapping) | unresolved, see findings below |

### FINDINGS FOR BACKLOG

- **File-path drift (now resolved in this split):** the legacy spec cites `crates/mcp-fs/src/search/*` and `crates/mcp-fs/src/tools/search_semantic.rs`; actual location is `crates/core/src/search/*` and `crates/core/src/tools/search_semantic.rs`. No code change needed, documentation-only drift, now carried correctly in this spec/design pair.
- **No failure counter for auto-indexing (legacy TBD-701):** indexing failures are `tracing` log lines only; `IndexStats` has no failure count or last-failure timestamp. A persistently failing embedding endpoint produces a silently stale index with no operator-visible signal beyond log volume. Candidate backlog item: add a failure counter/timestamp to `IndexStats`.
- **Unbounded detached indexing tasks (legacy TBD-702):** no semaphore or queue bounds the number of in-flight detached indexing tasks; a write burst spawns a burst of tasks. Candidate backlog item: bound concurrency with a `tokio::sync::Semaphore`.
- **No throttle/progress for mode-change rebuild (legacy TBD-703):** a full rebuild after a mode change is unbounded in duration and competes with live traffic, with no progress signal beyond `search.status` chunk counts. Candidate backlog item: throttle and/or progress reporting for background rebuilds.
- **Unverified test-coverage-mapping claim (legacy TBD-704):** the claim that `search/e2e.rs` (2960 lines) already covers most of this spec is based on line-count proportion, not on an actual test-to-FR mapping. Not spot-checked in this pass (out of scope for a doc-only retro-spec split); recorded as open for whoever next touches the search test suite.
