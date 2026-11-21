# US-0264: an index update that is refused or fails keeps the old entries

> Spec: SPEC-0100 (sdlc/specs/SPEC-0100-derived-metadata-rebuilds-after-the-file-settles/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 5
> Closes spec: no

## Objective
Give every search engine a guarded update: the embedding and chunking happen first, a caller supplied check runs right before the commit, and a refused or failed update leaves the previous entries untouched. The combined engine updates the vector side (which can fail) before the BM25 side. This is what later lets a rebuild be discarded without a trace.

## Requirements covered
- SPEC-0100/FR-NEW-011 (partial: the guard hook and the engine ordering): WHILE a rebuild of a file waits or runs THE system SHALL keep serving the previous search entries of that file unchanged.
- SPEC-0100/FR-NEW-012 (partial: the previous entries stay when an update fails): IF a rebuild fails THEN THE system SHALL set its job to `failed` with the reason, keep the previous search entries, leave the write that caused it successful, and SHALL NOT retry it on its own.
- SPEC-0100/FR-NEW-020 (partial: Tantivy and the combined engine replace entries in one step): WHEN a rebuild succeeds THE system SHALL replace the previous search entries of the file in one step.

## Independent Test
Run `cargo test -p fsa-core --lib search:: -- guarded`. The tests build a Tantivy backend and a combined backend over a stub embedding endpoint, index `alpha`, then call `index_path_checked` with a guard that returns false (and, separately, with the endpoint down) and assert that a BM25 query still returns the `alpha` chunk and none of the new text.

## Design excerpt
### Decisions
- DEC-008 (SPEC-0100): Add `SearchBackend::index_path_checked(..., still_current)` as a provided method (default ignores the guard and calls `index_path`); the four engines and Combined override it. `index_path` keeps its signature. **Obliged by:** SPEC-0100/FR-NEW-011, FR-NEW-013, FR-NEW-020. **Rationale:** the guard must run after embedding and before the commit; callers and the test `RecordingBackend` stay untouched. **Alternatives considered:** change `index_path` (touches `search.index`, `full_index`, tests), a prepare/commit split (more surface). **Source:** existing code. **Evidence:** `search/mod.rs:52-62`, `search/indexer.rs:455`.
- DEC-009 (SPEC-0100): Atomic replacement per engine: Tantivy already deletes and adds in one commit (`bm25_sqlite.rs:109-140`); PostgreSQL BM25, pgvector and sqlite-vec wrap delete plus inserts in one transaction; Combined runs the vector engine (embedding, can fail) first and BM25 second. **Obliged by:** SPEC-0100/FR-NEW-011, FR-NEW-012, FR-NEW-020. **Rationale:** a failure before any commit leaves every engine on the old entries. **Alternatives considered:** two phase across engines (needs staging tables). **Source:** existing code. **Evidence:** `search/bm25_pg.rs:56-90`, `search/vector_pg.rs:80-125`, `search/vector_sqlite.rs:93-170`, `search/mod.rs:252-262`.
- DEC-019 (SPEC-0100): Failure reason: an error whose text starts with `embedding request failed` or `embedding endpoint returned` becomes `embedding service unreachable`; any other error is `<ERR_CODE>: <message>`. **Obliged by:** SPEC-0100/FR-NEW-012. **Rationale:** matches the two failure texts of `embed`. **Alternatives considered:** a typed error (touches the whole `ToolError`). **Source:** existing code. **Evidence:** `search/embedding.rs:33-41`.
### Data model
None.
### Contracts
`SearchBackend::index_path_checked(&self, volume_id: &str, path: &str, text: &str, chunk_size: usize, chunk_overlap: usize, still_current: &(dyn Fn() -> bool + Send + Sync)) -> Result<Option<usize>>`; provided default calls `index_path` and wraps the count in `Some`. `None` means the guard refused and nothing was stored. `index_path` keeps its signature and means "always current".
### Paths
| Path | Action |
|---|---|
| `crates/core/src/search/mod.rs` | modify: trait method; `CombinedBackend::index_path_checked` runs `vector` first, then `bm25` |
| `crates/core/src/search/bm25_sqlite.rs` | modify: override; the guard is evaluated inside the `write_lock`, before `writer.commit()` |
| `crates/core/src/search/bm25_pg.rs` | modify: override; guard evaluated before the delete |
| `crates/core/src/search/vector_pg.rs` | modify: override; guard evaluated after the embeddings, before the delete |
| `crates/core/src/search/vector_sqlite.rs` | modify: override; guard evaluated after the embeddings, before the delete |
### Existing patterns
Embedding first, store second: `search/vector_pg.rs:80-125` computes every embedding before touching the database; copy that order. The Tantivy delete plus add in one commit is `search/bm25_sqlite.rs:109-140`. The stub embedding server pattern is in `search/e2e.rs` (search for `embedding`). A backend double is `RecordingBackend` in `search/indexer.rs:455`, which must keep compiling without change.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### Story test US-0264/T1: a refused guard stores nothing (Tantivy)
- Given Tantivy index with `/a.md` = `alpha beta`
- When `index_path_checked("v", "/a.md", "gamma delta", 1000, 100, &|| false)`
- Then the result is `Ok(None)`; `query_bm25("alpha")` returns `/a.md`; `query_bm25("gamma")` returns nothing
### Story test US-0264/T2: the combined engine keeps BM25 when the vector side fails
- Given a combined backend (Tantivy plus sqlite-vec) with `/a.md` indexed as `alpha`, then the embedding endpoint is stopped
- When `index_path_checked("v", "/a.md", "gamma", 1000, 100, &|| true)`
- Then the result is `Err`; `query_bm25("alpha")` still returns `/a.md`; `query_bm25("gamma")` returns nothing
### Story test US-0264/T3: the default method ignores the guard
- Given `RecordingBackend`
- When `index_path_checked(..., &|| false)`
- Then its `index_path` ran once and the result is `Ok(Some(n))`

## Constraints
- Do not change `index_path` or any caller of it.
- Do not add a dependency.
- The guard is a plain synchronous closure; never hold a lock across an await except the existing Tantivy `write_lock`.
- Transactions on PostgreSQL and sqlite-vec belong to US-0265.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit; main only receives the whole squash at landing.
- [ ] Non regression: the tests of the touched modules keep passing unchanged, except those named under Paths as modified.
- [ ] Docs updated: `.agent_docs/search.md`: one paragraph on the guarded update and the vector first order.
- [ ] Self review: every requirement above is met by a named test; every error text equals the design contract; no dash used as punctuation in code, comments or docs; no secret logged.

- [ ] One commit `US-0264: an index update that is refused or fails keeps the old entries` on the work branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0100` and the requirement ids.
