# US-0265: PostgreSQL and sqlite vec replace a file entries in one transaction

> Spec: SPEC-0100 (sdlc/specs/SPEC-0100-derived-metadata-rebuilds-after-the-file-settles/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0264
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Wrap the delete and the inserts of one file in a single transaction on the three engines that today run them one statement at a time, so a reader sees either all the old chunks or all the new ones and a failure halfway rolls back.

## Requirements covered
- SPEC-0100/FR-NEW-011 (partial: replacement is one step on PostgreSQL and sqlite vec): WHILE a rebuild of a file waits or runs THE system SHALL keep serving the previous search entries of that file unchanged.
- SPEC-0100/FR-NEW-012 (partial: a failure halfway leaves the old entries): IF a rebuild fails THEN THE system SHALL set its job to `failed` with the reason, keep the previous search entries, leave the write that caused it successful, and SHALL NOT retry it on its own.
- SPEC-0100/FR-NEW-020 (partial: PostgreSQL BM25, pgvector and sqlite vec replace entries in one step): WHEN a rebuild succeeds THE system SHALL replace the previous search entries of the file in one step.

## Independent Test
Run `cargo test -p fsa-core --lib search:: -- one_transaction`. The sqlite-vec test makes the second insert fail (a vector of the wrong dimension) and asserts the old chunks are still returned. The PostgreSQL tests run with `MCPFS_TEST_PG_DSN` set (see `.agent_docs/testing.md`) and assert the same for `search_fts` and `search_chunks`.

## Design excerpt
### Decisions
- DEC-009 (SPEC-0100): Atomic replacement per engine: Tantivy already deletes and adds in one commit (`bm25_sqlite.rs:109-140`); PostgreSQL BM25, pgvector and sqlite-vec wrap delete plus inserts in one transaction; Combined runs the vector engine (embedding, can fail) first and BM25 second. **Obliged by:** SPEC-0100/FR-NEW-011, FR-NEW-012, FR-NEW-020. **Rationale:** a failure before any commit leaves every engine on the old entries. **Alternatives considered:** two phase across engines (needs staging tables). **Source:** existing code. **Evidence:** `search/bm25_pg.rs:56-90`, `search/vector_pg.rs:80-125`, `search/vector_sqlite.rs:93-170`, `search/mod.rs:252-262`.
### Data model
No schema change.
### Contracts
No signature change: the overrides of `index_path_checked` from US-0264 keep their contract.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/search/bm25_pg.rs` | modify: `pool.begin()`, delete, inserts, `commit()` |
| `crates/core/src/search/vector_pg.rs` | modify: same, after the embeddings |
| `crates/core/src/search/vector_sqlite.rs` | modify: `unchecked_transaction()` around delete and inserts inside the `spawn_blocking` closure |
### Existing patterns
sqlx transaction use elsewhere: `search/vector_pg.rs` lines 80-125 show the statement sequence to wrap; for rusqlite, `Connection::unchecked_transaction` works through the shared `&Connection` held by the mutex guard (`vector_sqlite.rs:100-110`). Rollback on error is the drop of the transaction.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### Story test US-0265/T1: sqlite vec rolls back a half replacement
- Given sqlite-vec backend with `/a.md` indexed as 3 chunks of `old`
- When `index_path` is called with text that yields 3 chunks and the embedding of the third chunk has the wrong dimension (stub returns 3 floats instead of the configured size)
- Then the call returns `Err`; `stats` still reports 3 vector chunks; `query_vector("old")` returns `/a.md` with the old chunk text
### Story test US-0265/T2: PostgreSQL BM25 and pgvector roll back (opt in suite)
- Given `MCPFS_TEST_PG_DSN` set and the same setup on `search_fts` and `search_chunks`, with the second insert forced to fail by a NUL byte in the chunk text
- When `index_path` runs
- Then `Err`, and the old rows are still counted (`stats`) and returned by a query

## Constraints
- Do not hold a transaction while calling the embedding endpoint (embed first, as today).
- Do not change the table DDL.
- Do not touch Tantivy (already one commit).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit; main only receives the whole squash at landing.
- [ ] Non regression: the tests of the touched modules keep passing unchanged, except those named under Paths as modified.
- [ ] Docs updated: `.agent_docs/search.md`: note that entry replacement is one transaction per engine.
- [ ] Self review: every requirement above is met by a named test; every error text equals the design contract; no dash used as punctuation in code, comments or docs; no secret logged.

- [ ] One commit `US-0265: PostgreSQL and sqlite vec replace a file entries in one transaction` on the work branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0100` and the requirement ids.
