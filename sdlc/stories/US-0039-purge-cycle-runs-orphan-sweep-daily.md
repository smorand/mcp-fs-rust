# US-0039: The purge cycle runs the orphan sweep once a day

> Spec: SPEC-0034 (sdlc/specs/SPEC-0034-sweep-unreferenced-blobs/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0038
> min_tier: 2
> Files touched: 4
> Closes spec: yes

## Objective
Wire the sweep into the purge cycle at most once every 24 hours per project per process, isolate its failures from the rest of the cycle, report its counters in `CycleSummary` and the cycle log, and document it next to the existing GC rule.

## Requirements covered
#### SPEC-0034/DR-007 [EARS-S]: Once a day per project
> WHILE less than 24 hours have passed since the last sweep of a project by this process THE purge cycle SHALL skip that project's sweep; a process that has not swept a project yet SHALL sweep it on its first cycle.
- The last run time is recorded when the sweep of the project completes or fails, so a failing project is retried after 24 hours, not every cycle.

#### SPEC-0034/DR-008 [EARS-O]: Failures are isolated
> IF listing, a reference read or a delete fails THEN the sweep SHALL log a warning with the project and the error, SHALL skip that object (or that project, for a listing or bulk reference read failure), and SHALL NOT abort the cycle, another project or another sweep step.
- A backend answering `ERR_NOT_SUPPORTED` to `list` is skipped silently (no warning).

#### SPEC-0034/DR-009 [EARS-U]: Counters
> `CycleSummary` SHALL carry `orphan_blobs_swept` (objects deleted), `orphan_bytes_swept` (sum of their listed sizes) and `orphan_breakers_tripped`, summed over projects, and the purge loop log line SHALL include all three.

#### SPEC-0034/DR-010 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.


## Independent Test
`cargo test -p mcp-fs-core --lib purge::tests` passes with DT-022, DT-023, DT-025, DT-026; the full gate (DT-028) passes with no edit to any existing test and `tool-contract-golden.json` unchanged.

## Design excerpt
### Decisions
- **(SPEC-0034) DEC-005:** The schedule lives in `StoreManager`, keyed by project id, and the sweep takes `now` as a parameter (`sweep_project_orphans_at`, mirroring `sweep_project_files_at` and `sweep_project_artifacts_at`, `purge.rs:268`); `run_cycle` keeps its signature and passes the current time. **Alternatives considered:** a process global static, rejected because parallel tests would share it; a new `run_cycle` parameter, rejected because 13 existing tests and the loop call it (14 call sites, Section 2). **Implemented by:** DR-007. **Evidence:** `crates/core/src/storage/mod.rs:416-424`, `crates/core/src/purge.rs:210-215`.
- **(SPEC-0034) DEC-006:** The breaker outcome is a counter (`orphan_breakers_tripped`) asserted by tests, plus a warning log. **Alternatives considered:** asserting the log line, rejected because it needs a log capture dependency the workspace does not have. **Implemented by:** DR-006, DR-009. **Evidence:** `Cargo.toml` has no `tracing-test`.
### Assumptions (SPEC-0034)
- **Assumption:** one bucket per project (`crates/core/src/config.rs:945-947`, `crates/core/src/storage/mod.rs:211-216`); were two projects to share a bucket, the sweep would delete the other's blobs. Not configurable today.
- **Assumption:** a residual window of milliseconds remains between the final re check and the delete, on both backends (S3 has no conditional delete here). It is the window the existing delete at zero path already accepts (`volume.rs:170-172`, SPEC-0033 assumption).
- **Assumption:** single server process per database (BL-0016); the in memory schedule is per process.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/mod.rs` | `StoreManager` (`:416-424`): add `orphan_sweeps: Mutex<HashMap<String, f64>>` and accessor methods to read and record a project's last run |
| `crates/core/src/purge.rs` | `CycleSummary` (`:198-203`): add `orphan_blobs_swept`, `orphan_bytes_swept`, `orphan_breakers_tripped`; `run_cycle` (`:210-255`) calls the gated sweep after `sweep_project_artifacts` (`:248`); add `run_cycle_at(now, ...)` for tests, `run_cycle` keeps its signature (14 callers) |
| `crates/core/src/app.rs` | the cycle log line (`:223-227`) adds the three counters |
| `.agent_docs/architecture.md` | a paragraph next to "GC at refcount 0" (`:61-64`) describing the sweep, its 24 h age, the breaker and the excluded `git:` keys |
### Existing patterns
- The existing per step isolation in `run_cycle` (`purge.rs:216-254`): a failing step logs `tracing::warn!` and the cycle continues.
- `run_cycle` call sites stay unchanged: `app.rs:215`, 11 tests in `purge.rs`, `tools/teardown_e2e.rs:408`, `tools/trash_search_e2e.rs:169`.

## Acceptance tests
> 100% must pass, run through `make check`. Shared fixture: one project with a local blob store in a temp dir, built like the `run_cycle` tests (`crates/core/src/purge.rs:1013`); ages set with `std::fs::File::set_modified`; `now` injected. `A` a referenced content hash, `O` an orphan content hash 25 h old, `Y` an orphan 1 h old.

| Test | Category | Requirement | Location | Given / When / Then | Red before because |
|---|---|---|---|---|---|
| SPEC-0034/DT-022 | behavior | DR-007 | `purge.rs` tests | fresh `StoreManager`, old orphan: swept on the first cycle; a second orphan added, cycle at +23 h: kept; cycle at +24 h 1 min: swept; a new `StoreManager` over the same store sweeps on its first cycle | no sweep |
| SPEC-0034/DT-023 | failure | DR-008, DR-001 | `purge.rs` tests | project P1 whose backend `list` fails, P2 with an old orphan and a stale file: `run_cycle` returns `Ok`, P2's orphan swept, P2's existing purge counts unchanged | no sweep |
| SPEC-0034/DT-025 | behavior | DR-009 | `purge.rs` tests | DT-014 setup with `O` 7 bytes, `export:t2` 11 bytes, `.tmp-x` 5 bytes: `orphan_blobs_swept == 3`, `orphan_bytes_swept == 23` | fields do not exist |
| SPEC-0034/DT-026 | edge | DR-001, DR-006 | `purge.rs` tests | empty bucket: counters 0, `orphan_breakers_tripped == 0`, cycle `Ok` | no sweep |
| SPEC-0034/DT-028 | guardrail | DR-010 | full suite | 8.1 green with no edit to an existing test, `tool-contract-golden.json` unchanged | n/a (invariance) |

## Constraints
- `run_cycle` keeps its signature (DEC-005); no new setting, no new CLI flag (owner decision (i)).
- The schedule is in memory only; no table, no migration.
- Record the last run when the sweep completes or fails, so a failing project is retried after 24 h, not every cycle.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code, in `.implement/US-0039-red.log`.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] DEBT: The existing test suite passes **unmodified** (exceptions: none, 0):

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

- [ ] Docs updated: `.agent_docs/architecture.md`.
- [ ] Self review: full (state the time and space complexity of the hot path).
- [ ] Closes spec: the full acceptance set of SPEC-0034 green (DT-010..DT-028; DT-012 needs MinIO and is run when `MCPFS_MINIO_SECRET_KEY` is set, otherwise reported as not run), and every DR-001..DR-010 cited by a done or landed story.
- [ ] One commit `US-0039: The purge cycle runs the orphan sweep once a day`, body `Spec: SPEC-0034` and the DR ids.
