# Batch the tag row writes of the reconcile: technical debt design

> Id: SPEC-0073
> Nature: DEBT
> Depth: S
> Depth evidence: 2 files, 1 debt item, no public contract change, no persisted schema change
> Status: specified
> Generated on: 2026-11-02
> Behavior change: none
> From backlog: BL-0102
> Depends on: SPEC-0071
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt
### DBT-001: one database commit per incoming tag
- **Current structure:** `reconcile_incoming_tags` awaits `entry.db.set_ref(...)` once per added tag inside its loop (`crates/core/src/tools/git_tags.rs:446-447`). `set_ref` is a single autocommit upsert (`crates/core/src/git/db.rs:371-383`), so n added tags cost n commits. The function runs at three sites: clone (`crates/core/src/tools/git.rs:3375`), fetch (`:3931`) and pull (`:6817`). The function also runs `list_refs` before its loop even when `incoming` is empty (`git_tags.rs:433-438`).
- **Cost paid today:** 2 000 sequential commits for the 2 000 tags of `e2e_tag_048_two_thousand_tags` (`git.rs:25901`), a test that takes 6.59 s end to end in a run of 2026-11-02 (fetch and 2 000 tags built included; the reconcile share is not separated, which is why the gain is stated as a count of commits, Section 2, NFR).
- **Target structure:** `RelationalGitDb` gets `set_refs(&self, refs: &[(String, String)])` which writes every row in one transaction (`RelationalDb::begin`, `crates/core/src/storage/rel/mod.rs:272`, `RelationalTx::commit` `:292`). `reconcile_incoming_tags` collects the rows to add, calls `set_refs` once, and returns early before `list_refs` when `incoming` is empty. The on disk ref writes (SPEC-0071's `sync_disk_ref`) run after the commit.
- **Observable behavior:** unchanged for every run that succeeds (DEC-002 for a failure).

## 2. Perimeter: the Current State
| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/git/db.rs` | add `set_refs`; unit tests | 1 new function |
| `crates/core/src/tools/git_tags.rs` | `reconcile_incoming_tags` collects, batches, early return on empty | 1 loop (`:446-447`), 1 listing (`:433`) |
| `.agent_docs/git.md` | one sentence in the tag section: the reconcile writes in one transaction | 1 |
| `sdlc/backlog/BL-0102-*.md` | deleted in the landing commit | 1 |

**Non source occurrences:** none (`rg 'set_ref\(' --glob '!crates/**' --glob '!.agent_docs/**'` returns 0 hits besides the sdlc documents).
**Other `set_ref` callers:** untouched; recounted: `rg -c 'set_ref\(' crates/core/src/tools/git.rs crates/core/src/tools/git_tags.rs` lists the single writes of branches, HEAD and tags. Only the reconcile loop changes.
**Contracts touched:** `RelationalGitDb::set_refs` is new and internal; `reconcile_incoming_tags` keeps its signature and its three callers.
**Guardrail coverage:** the git lib tests including `e2e_tag_048_two_thousand_tags` (`git.rs:25901`) and the reconcile tests that exercise added, kept local and identical tags (ASSUMED: count not recounted; the command below runs them). **Test command:** `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test -p mcp-fs-core --lib git`
**Total:** 2 source files, 2 call changes, 1 new function, 1 doc line.
**Out of scope:** BL-0101 (clone hydration) stays in the backlog; its notes now record that the reconcile writes the on disk tag ref after the row, which needs the objects hydrated.

## 3. Class sweep
Not applicable (`Security: n/a`).

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-U]: one batch write
> The git index store SHALL write the tag rows of one reconcile through one call that uses one transaction.
- **Old form:** `entry.db.set_ref(&tag_ref, &tag.oid.to_string(), false).await?;` inside the loop.
- **New form:** `entry.db.set_refs(&rows).await?;` once, where `rows: Vec<(String, String)>` holds `(refs/tags/<name>, <oid>)` in the incoming order; the rows are written non symbolic.
- **Applies to:** `git_tags.rs:446-447`, `git/db.rs`.
- **Casing convention:** snake_case, the codebase's.

#### DR-002 [EARS-O]: empty input
> IF `incoming` is empty THEN THE reconcile SHALL return `(vec![], vec![])` without any database read or write.
#### DR-003 [EARS-UB]: no write in a loop
> The reconcile SHALL NOT call `set_ref` inside a loop.

### 4.2 Invariance
#### DR-004 [EARS-U]: behavior invariance
> The git tools SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change: the same `tags_added` and `tags_kept_local` lists, the same stored rows, the same order.

### 4.3 Atomicity
#### DR-005 [EARS-O]: a failed batch leaves nothing
> IF writing one row of the batch fails THEN THE index store SHALL roll the whole batch back and `reconcile_incoming_tags` SHALL return that error with no tag row stored by this call.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `RelationalGitDb::set_refs` | function | no | introduced | n/a | n/a |
| `reconcile_incoming_tags` | function | no | signature unchanged | n/a | n/a |

**Dual maintenance semantics:** none.

## 6. Decisions
- **DEC-001:** one transaction for the whole reconcile. **Alternatives considered:** chunks of 500 rows (rejected by the owner: more code, keeps a partial state, no measured need); multi row `INSERT` statements (rejected: a dialect fragment per engine for a gain a transaction already gives). **Rationale:** the cost is the commit, not the statement. **Implemented by:** DR-001. **Evidence:** `db.rs:371-383`.
- **DEC-002:** a failure while writing the rows now stores none of them; today it leaves the prefix written before the failure. **Alternatives considered:** keep the prefix (rejected: needs chunking or a savepoint per row). **Rationale:** no spec or test promises the partial state (`rg` of the three specs SPEC-0052, SPEC-0060, SPEC-0065 finds no partial write requirement; ASSUMED for tests, the non regression command checks). This is the only observable difference and only on a failing write; recorded in Section 7. **Implemented by:** DR-005.
- **DEC-003:** the on disk ref writes stay per tag, after the commit, through SPEC-0071's helper. **Alternatives considered:** inside the transaction (rejected: they are filesystem writes, a rollback cannot undo them). **Implemented by:** DR-001 ordering.
- **Assumption:** the 6.59 s figure is one run on a loaded laptop (load average about 5) and is a ceiling, not a benchmark.

## 7. Declared breaks
- **Contract:** none public. **Internal difference:** a failed write during a reconcile leaves no tag row (DEC-002), where it left a prefix. Affected: nobody who relies on the previous partial state; BL-0100 (open) concerns a different failure, after the reconcile.

## 8. Tests
### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**:
    export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace

### 8.2 Behavior tests
#### DT-001: three tags give three identical rows
- **Validates:** DR-001, DR-004. Given an empty project and the incoming tags `v1`, `v2`, `v3`, When the reconcile runs, Then `refs/tags/v1`..`v3` exist with the incoming ids, `tags_added` is `["v1","v2","v3"]`. Passes on arrival; named mutation: `set_refs` writes only the first row, DT-001 goes red.
#### DT-002: a failing row stores nothing
- **Validates:** DR-005. Given incoming tags `a`, `b`, `c` and a store whose write fails on `b`, When the reconcile runs, Then it returns the error and the index holds no `refs/tags/` row.
- **Red before:** `refs/tags/a` is stored today (prefix).
#### DT-003: empty input touches nothing
- **Validates:** DR-002. Given `incoming = []` and a counting store double, Then zero queries and zero writes were issued, and the result is `([], [])`.
- **Red before:** one `list_refs` query is issued today (`git_tags.rs:433`).
#### DT-004: 2 000 tags, one transaction
- **Validates:** DR-001. Given 2 000 incoming tags and a counting store double, Then `begin` was called once, `commit` once, and no autocommit write was issued.
- **Red before:** 2 000 autocommit writes today.

### 8.3 Structural tests
#### DT-010: no `set_ref` in the reconcile
- **Asserts:** the body of `reconcile_incoming_tags` contains no `set_ref(`. **Method:** a unit test slicing the function body from `include_str!("git_tags.rs")`.

### 8.4 Existing tests to modify
None.

## 9. Implementation order
1. Tests DT-001 to DT-004 and DT-010 (DT-002 to DT-004 red; the store double lands with them).
2. Add `set_refs` and switch the reconcile; tests green, suite green.
3. Update `.agent_docs/git.md`; `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.
Order note: apply after SPEC-0071 (same function), or resolve the small merge by hand; either order keeps the suite green.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied:** none.
