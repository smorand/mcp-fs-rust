# US-0148: batch the tag row writes of the reconcile

> Spec: SPEC-0073 (sdlc/specs/SPEC-0073-batch-the-tag-row-writes/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0147
> min_tier: 2
> Files touched: 3
> Closes spec: yes

## Objective
`reconcile_incoming_tags` commits one database transaction per added tag. Write all its rows in one transaction, return early on empty input without any read, and keep every observable result the same.

## Requirements covered
- SPEC-0073/DR-001: The git index store SHALL write the tag rows of one reconcile through one call that uses one transaction.
- SPEC-0073/DR-002: IF `incoming` is empty THEN THE reconcile SHALL return `(vec![], vec![])` without any database read or write.
- SPEC-0073/DR-003: The reconcile SHALL NOT call `set_ref` inside a loop.
- SPEC-0073/DR-004: The git tools SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change: the same `tags_added` and `tags_kept_local` lists, the same stored rows, the same order.
- SPEC-0073/DR-005: IF writing one row of the batch fails THEN THE index store SHALL roll the whole batch back and `reconcile_incoming_tags` SHALL return that error with no tag row stored by this call.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green including DT-001 to DT-004 and DT-010; with `set_refs` writing only its first row, DT-001 fails.

## Design excerpt
### Decisions
- DEC-001: one transaction for the whole reconcile. Alternatives considered: chunks of 500 rows (rejected by the owner: more code, keeps a partial state, no measured need); multi row `INSERT` statements (rejected: a dialect fragment per engine for a gain a transaction already gives). Rationale: the cost is the commit, not the statement. Implemented by: DR-001. Evidence: `db.rs:371-383`.
- DEC-002: a failure while writing the rows now stores none of them; today it leaves the prefix written before the failure. Alternatives considered: keep the prefix (rejected: needs chunking or a savepoint per row). Rationale: no spec or test promises the partial state (`rg` of the three specs SPEC-0052, SPEC-0060, SPEC-0065 finds no partial write requirement; ASSUMED for tests, the non regression command checks). This is the only observable difference and only on a failing write; recorded in Section 7. Implemented by: DR-005.
- DEC-003: the on disk ref writes stay per tag, after the commit, through SPEC-0071's helper. Alternatives considered: inside the transaction (rejected: they are filesystem writes, a rollback cannot undo them). Implemented by: DR-001 ordering.
- DEC-004: `pub async fn set_refs(&self, refs: &[(String, String)]) -> Result<()>` on `RelationalGitDb`: `let mut tx = self.db.begin().await?;` then one upsert per row (the SQL of `set_ref`, `symbolic` bound to 0), `tx.commit().await?`; an error returns before `commit`, so dropping the handle rolls back (`rel/mod.rs:271`). Obliged by: DR-001, DR-005. Rationale: `begin()` and not `run_retrying`: the upserts are idempotent so either works, and no transient failure of this write was ever shown; a plain transaction is the least code. Alternatives considered: `run_retrying` (rejected: no retry need shown; adds a closure); multi row insert (rejected by DEC-001). Source: design. Evidence: `db.rs:371-383`, `rel/mod.rs:268-274`.
- DEC-005: `reconcile_incoming_tags` order: return `(vec![], vec![])` when `incoming` is empty (before `list_refs`); build `added` rows and `kept_local`; call `set_refs(&rows)` once if `rows` is non empty; then for each added tag call the on disk write (`sync_disk_ref` from SPEC-0071 when present, else the existing `let _ = repo.reference(...)`). Obliged by: DR-001, DR-002. Source: design. Evidence: `git_tags.rs:433-460`.
- DEC-006: one story (three files, five tests). Obliged by: n/a (no code impact): partitioning only. Source: design.
- DEC-007: red evidence: DT-002, DT-003, DT-004, DT-010 fail before the change; DT-001 passes on arrival and is proven by the named mutation of the spec. Obliged by: DR-005. Source: design.
- DEC-008: order against SPEC-0071: US-0148 depends on US-0147 in the queue; if US-0147 has landed, step 3 of DEC-005 calls `sync_disk_ref`; the story must not reintroduce a discarded `let _ = repo.reference(` in `reconcile_incoming_tags` (DT-010 of SPEC-0071 forbids it). Obliged by: DR-001. Source: design.
### Data model
None.
### Contracts
`pub async fn set_refs(&self, refs: &[(String, String)]) -> Result<()>` on `RelationalGitDb` in `crates/core/src/git/db.rs`: one `begin()`, one upsert per `(name, target)` with `symbolic = 0`, one `commit()`; an error returns before commit. `reconcile_incoming_tags` (`git_tags.rs:428+`) keeps its signature and callers (`git.rs:3375`, `:3931`, `:6817`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/db.rs` | modify (`set_refs`, `CountingRel` test wrapper, DT-001, DT-002) |
| `crates/core/src/tools/git_tags.rs` | modify (`reconcile_incoming_tags`, DT-003, DT-004, DT-010) |
| `.agent_docs/git.md` | modify (one sentence) |
### Existing patterns
`set_ref` at `crates/core/src/git/db.rs:371-383`; `begin()` and `commit()` at `crates/core/src/storage/rel/mod.rs:268-292`; the private `CountingDb` at `storage/rel/mod.rs:395-452` as the shape of a counting wrapper; `open_in_memory` at `db.rs:233`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.


### SPEC-0073/DT-001: three tags give three identical rows
- Level / Driver / Location: integration / `set_refs` or `reconcile_incoming_tags` over `CountingRel` / `db.rs` and `git_tags.rs` tests; DT-010 structural by `include_str!`
- **Validates:** DR-001, DR-004. Given an empty project and the incoming tags `v1`, `v2`, `v3`, When the reconcile runs, Then `refs/tags/v1`..`v3` exist with the incoming ids, `tags_added` is `["v1","v2","v3"]`. Passes on arrival; named mutation: `set_refs` writes only the first row, DT-001 goes red.
- Cleanup: temp directories are dropped with the test.

### SPEC-0073/DT-002: a failing row stores nothing
- Level / Driver / Location: integration / `set_refs` or `reconcile_incoming_tags` over `CountingRel` / `db.rs` and `git_tags.rs` tests; DT-010 structural by `include_str!`
- **Validates:** DR-005. Given incoming tags `a`, `b`, `c` and a store whose write fails on `b`, When the reconcile runs, Then it returns the error and the index holds no `refs/tags/` row.
- **Red before:** `refs/tags/a` is stored today (prefix).
- Cleanup: temp directories are dropped with the test.

### SPEC-0073/DT-003: empty input touches nothing
- Level / Driver / Location: integration / `set_refs` or `reconcile_incoming_tags` over `CountingRel` / `db.rs` and `git_tags.rs` tests; DT-010 structural by `include_str!`
- **Validates:** DR-002. Given `incoming = []` and a counting store double, Then zero queries and zero writes were issued, and the result is `([], [])`.
- **Red before:** one `list_refs` query is issued today (`git_tags.rs:433`).
- Cleanup: temp directories are dropped with the test.

### SPEC-0073/DT-004: 2 000 tags, one transaction
- Level / Driver / Location: integration / `set_refs` or `reconcile_incoming_tags` over `CountingRel` / `db.rs` and `git_tags.rs` tests; DT-010 structural by `include_str!`
- **Validates:** DR-001. Given 2 000 incoming tags and a counting store double, Then `begin` was called once, `commit` once, and no autocommit write was issued.
- **Red before:** 2 000 autocommit writes today.
- Cleanup: temp directories are dropped with the test.

### SPEC-0073/DT-010: no `set_ref` in the reconcile
- Level / Driver / Location: integration / `set_refs` or `reconcile_incoming_tags` over `CountingRel` / `db.rs` and `git_tags.rs` tests; DT-010 structural by `include_str!`
- **Asserts:** the body of `reconcile_incoming_tags` contains no `set_ref(`. **Method:** a unit test slicing the function body from `include_str!("git_tags.rs")`.
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: any `sdlc-spec.md`, receive-pack and upload-pack code, `tool-contract-golden.json`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: the other stories of the queue.

## Definition of done
- [ ] RED evidence recorded for every new test that can fail before the change; tests that guard behavior that already exists are proven by the mutation named in the story.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all tests of `cargo test -p mcp-fs-core --lib git` keep passing unchanged. The SPEC-0071 helper, if landed, is used for the on disk write and its DT-010 stays green.
- [ ] The existing suite passes unmodified: `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace`; exceptions: none (0 existing tests modified).
- [ ] Docs updated: `.agent_docs/git.md` (the reconcile writes its rows in one transaction).
- [ ] Self review: full.
- [ ] Closes spec: the spec's full acceptance set green and every requirement of SPEC-0073 cited by at least one story with status done or landed.
- [ ] One commit `US-0148: batch the tag row writes of the reconcile`, body `Spec: SPEC-0073` and the requirement ids.
