# US-0046: BT-020 waits for indexing before its baseline

> Spec: SPEC-0039 (sdlc/specs/SPEC-0039-deterministic-bt020-baseline/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: yes

## Objective
`spec_0024_bt_020_other_projects_untouched_tantivy` fails under full suite load because its baseline is taken before the docx companion is indexed in the background. Replace the "two reads agree" settle with an exact count wait.

## Requirements covered
- SPEC-0039/DR-001: `others_untouched` SHALL call `f.await_indexed(OLIVIA, project, 2)` for each of `ATLAS` and `ATLAS2` after documentizing and before taking the `ATLAS2` baseline with `snapshot`.
- SPEC-0039/DR-002: `project_removal_e2e.rs` SHALL NOT contain `settled_snapshot`.
- SPEC-0039/DR-003: byte identical observable output; every assertion of `others_untouched` (`:779-781`) unchanged; `tool-contract-golden.json` unchanged.

## Independent Test
With the PostgreSQL DSN set, `cargo test -p mcp-fs-core --lib spec_0024_bt_020_other_projects_untouched` passes both tests, including under the DT-001 delay; `rg -c settled_snapshot crates/core/src/tools/project_removal_e2e.rs` prints nothing.

## Design excerpt
### Decisions
- DEC-001: wait on the exact index count with `Fixture::await_indexed` (`project_removal_e2e.rs:235-247`, already used by `seed` `:281-283`).
- DEC-002: prove the race with a temporary, uncommitted 1 s delay at the start of the task spawned in `ProjectIndexer::on_write` (`search/indexer.rs:113-124`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/project_removal_e2e.rs` | `:772-775` new wait; delete `settled_snapshot` `:750-762` |
### Existing patterns
- `seed` (`:274-292`) waits with `self.await_indexed(OLIVIA, project, 2)`.

## Acceptance tests
### SPEC-0039/DT-001: race reproduced, then closed
- Insert `tokio::time::sleep(std::time::Duration::from_secs(1)).await;` first in the task spawned by `on_write`. Old test: `atlas2 changed` panic. New test, same delay: both pass. Remove the delay; `git diff -- crates/core/src/search/indexer.rs` empty.

### SPEC-0039/DT-002: no heuristic left
- `rg -c settled_snapshot crates/core/src/tools/project_removal_e2e.rs` prints nothing; `rg -n "await_indexed\(OLIVIA, project, 2\)"` finds the call in `others_untouched`.

### SPEC-0039/DT-003: guardrail
- Non regression command green; `git diff main -- tool-contract-golden.json` empty.

## Constraints
- Committed change only in `project_removal_e2e.rs`; `indexer.rs` unchanged in the commit.
- Assertions `:779-781` unchanged.

## Definition of done
- [ ] RED evidence: DT-001 old test under the delay, in `.implement/US-0046-red.log`.
- [ ] Every acceptance test green; `make check` green.
- [ ] DEBT: the existing suite passes unmodified, exceptions: 1, `others_untouched` wait mechanism only (declared in SPEC-0039 Section 8.4).
- [ ] Non regression command: `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] Closes spec: DT-001..DT-003 green, DR-001..DR-003 cited.
- [ ] One commit `US-0046: BT-020 waits for indexing before its baseline`, body `Spec: SPEC-0039` and the DR ids.
