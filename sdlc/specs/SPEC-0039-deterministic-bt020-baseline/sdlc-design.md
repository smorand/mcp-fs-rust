# BT-020 takes its baseline after indexing has finished: technical debt design

> Id: SPEC-0039
> Nature: DEBT
> Depth: S
> Depth evidence: one test function (`others_untouched`, `crates/core/src/tools/project_removal_e2e.rs:764-782`), one file, no production code, no contract, no persisted change
> Status: implemented
> Generated on: 2026-10-18
> Behavior change: none
> From backlog: BL-0065 (filed as `BUG (test only)`; routed DEBT by owner decision 2026-10-18: no product authority is violated, the test's timing assumption is)
> Depends on: SPEC-0024 (implemented, BT-020)
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt
### DBT-001: BT-020 baseline races a background index write
- **Current structure:** `others_untouched` seeds both projects, documentizes `/memo.docx` in each (`project_removal_e2e.rs:768-771`), which writes a Markdown companion from the fixture's stub converter (`:161`) whose indexing is a detached task (`after_companion`, `search/indexer.rs:235-247`; `on_write` spawns, `:113-124`). The baseline comes from `settled_snapshot` (`:750-762`): the first two reads 300 ms apart that agree.
- **Cost paid today:** under full suite load the companion is indexed after the baseline (`bm25_docs` 1 then 2), so `spec_0024_bt_020_other_projects_untouched_tantivy` fails with `atlas2 changed` (`:780`) though nothing touched atlas2; observed in the US-0045 `make check` (2026-10-18), green in 3 of 3 isolated runs.
- **Target structure:** wait for the exact expected index count with the existing `Fixture::await_indexed` (`:235-247`) before one `snapshot`; `settled_snapshot` removed.
- **Observable behavior:** unchanged (test only).

## 2. Perimeter: the Current State
| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/tools/project_removal_e2e.rs` | `others_untouched` `:772-775`: after the documentize loop, `f.await_indexed(OLIVIA, project, 3)` for each of `ATLAS`, `ATLAS2`, then `let before = snapshot(&f, ATLAS2).await;`; delete `settled_snapshot` `:750-762` (its only call site is `:775`) | 1 test body, 1 helper removed |

**Non source occurrences:** none (`rg -n settled_snapshot` outside this file returns nothing).
**Contracts touched:** none.
**Guardrail coverage:** the two callers `spec_0024_bt_020_other_projects_untouched_tantivy` (`:788`) and `_pg` (`:794`). **Test command:** `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
**Total:** 1 file.

## 3. Class sweep
n/a.

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-U]: Deterministic baseline
> `others_untouched` SHALL call `f.await_indexed(OLIVIA, project, 3)` for each of `ATLAS` and `ATLAS2` after documentizing, before taking the `ATLAS2` baseline with `snapshot`, and SHALL assert that baseline status holds `bm25_docs` 3.
- **Old form:** `let before = settled_snapshot(&f, ATLAS2).await;`
- **New form:** `for project in [ATLAS, ATLAS2] { f.await_indexed(OLIVIA, project, 3).await; }` then `let before = snapshot(&f, ATLAS2).await;` and `assert_eq!(before[2]["bm25_docs"], 3)`
- **Why 3:** `/plan.txt` (seed leaves 1, `:283`), `/memo.docx` itself (its bytes `PK docx` are UTF-8, so the write hook indexes it, `:769`) and its Markdown companion; measured settled value 3 for both projects (owner amendment 2026-10-19, US-0046 evidence). `await_indexed` sums `bm25_docs` and `vector_chunks` (`:228-231`); counts only rise here, so exact 3 is the settled state.

#### DR-002 [EARS-UB]: No heuristic settle
> `project_removal_e2e.rs` SHALL NOT contain `settled_snapshot`.

### 4.2 Invariance
#### DR-003 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change, every assertion of `others_untouched` (`:779-781`) SHALL be unchanged, and `tool-contract-golden.json` SHALL be unchanged.

### 4.3 Compatibility
None.

## 5. Compatibility plan
None.

## 6. Decisions
- **DEC-001:** Wait on the exact index count with `await_indexed`. **Alternatives considered:** a longer settle window (rejected: only makes the race rarer); returning and awaiting the background `JoinHandle`s (rejected: they are dropped inside production hooks, `indexer.rs:202`, `:219`, so it means threading handles out of the tool layer for a test). **Implemented by:** DR-001, DR-002. **Evidence:** `project_removal_e2e.rs:235-247`, `:281-283`.
- **DEC-002:** Prove the race with a temporary, uncommitted 1 s delay in `ProjectIndexer::on_write` (`indexer.rs:113-124`). **Alternatives considered:** loop the test N times under load (rejected: probabilistic, not a red signal). **Implemented by:** n/a, no code impact (evidence procedure for DT-001). **Evidence:** BL-0065.

## 7. Declared breaks
None.

## 8. Tests
### 8.1 Non regression: the success criterion
The existing test suite passes, unmodified except the 8.4 row:

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

### 8.2 Compatibility tests
None.

### 8.3 Structural and race tests
#### DT-001: race reproduced, then closed
- **Validates:** DR-001
- **Method:** temporarily insert `tokio::time::sleep(std::time::Duration::from_secs(1)).await;` as the first statement of the task spawned in `ProjectIndexer::on_write` (`indexer.rs:113-124`); with the new baseline assertion (`bm25_docs` 3) in place, run `cargo test -p mcp-fs-core --lib spec_0024_bt_020_other_projects_untouched` (PostgreSQL DSN set) with the old wait, then with the new wait; remove the delay and show `git diff -- crates/core/src/search/indexer.rs` empty.
- **Red before the fix because:** with the delay and the old `settled_snapshot` wait the baseline reads `bm25_docs` 1, so `assert_eq!(before[2]["bm25_docs"], 3)` fails; with `await_indexed(..., 3)` both tests pass under the same delay. (The original `atlas2 changed` assertion cannot go red under a fixed delay: baseline and final read both precede the delayed writes, measured 2026-10-19.)

#### DT-002: no heuristic left
- **Validates:** DR-002, DR-001
- **Method:** `rg -c settled_snapshot crates/core/src/tools/project_removal_e2e.rs` prints nothing; `rg -n "await_indexed\(OLIVIA, project, 2\)"` finds the call inside `others_untouched`.
- **Red before:** `settled_snapshot` is present (`:750`, `:775`).

#### DT-003: guardrail
- **Validates:** DR-003. 8.1 green; `git diff main -- tool-contract-golden.json` empty.

### 8.4 Existing tests to modify
| Test file | Test | Change | Justification |
|---|---|---|---|
| `crates/core/src/tools/project_removal_e2e.rs` | `others_untouched` (body of `spec_0024_bt_020_other_projects_untouched_tantivy` and `_pg`) | wait mechanism before the baseline, plus one added assertion that the baseline holds `bm25_docs` 3; assertions `:779-781` unchanged | the heuristic wait is the defect (DBT-001) |

## 9. Implementation order
1. Delay mutation on, run old test: RED (DT-001). 2. Rewrite the wait (DR-001, DR-002); same delay: GREEN; delay removed. 3. 8.1 green. One story.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE (inline, depth S): exact old and new forms; count 2 justified by cited seed and status sum; mutation procedure literal; one declared test edit; no open question; EARS clean; G13 not triggered; citations opened (`project_removal_e2e.rs:161`, `:228-247`, `:281-283`, `:750-782`, `:788`, `:794`, `indexer.rs:113-124`, `:199-247`) |

**Amendments applied:** none.

## Design additions (2026-10-19)

### File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/project_removal_e2e.rs` | `others_untouched` wait; `settled_snapshot` removed | DR-001, DR-002 | US-0046 |
| `crates/core/src/search/indexer.rs` | temporary delay for DT-001 only, never committed | DEC-002 | US-0046 |

### Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-001 | integration (mutation) | the two BT-020 tests under a 1 s indexing delay | `project_removal_e2e.rs:788`, `:794` | US-0046 |
| DT-002 | structural | `rg` on the test file | shell evidence in the commit body | US-0046 |
| DT-003 | guardrail | full suite | `make check` | US-0046 |

### Additional decisions
- **DEC-003:** One story at `min_tier: 2`. **Obliged by:** SPEC-0039/DR-001..DR-003. **Rationale:** one committed file, one function; not divisible. **Alternatives considered:** none viable. **Source:** design. **Evidence:** table above.

### Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0046 | BT-020 waits for indexing before its baseline | P1 | DR-001, DR-002, DR-003 | main | 2 | yes |

## Amendment (2026-10-19, owner authorized)
DR-001, DT-001, DT-002 and 8.4 corrected from 2 to 3 entries and given a baseline assertion, after US-0046 measured the settled count (3) and showed a fixed delay cannot turn `atlas2 changed` red (see `sdlc/stories/US-0046-bt020-deterministic-baseline.md`, Blocked).
