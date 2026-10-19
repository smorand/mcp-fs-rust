# Make the trash_entries write atomic with the rename it accompanies: technical debt design

> Id: SPEC-0044
> Nature: DEBT
> Depth: L
> Depth evidence: 3 modules (`storage/meta`, `core/fs_ops`, `purge`), 0 persisted change, 0 public contract change, 8 requirements. Counted L by the "3 or more modules" rule; one bounded context (trash recording).
> Status: specified
> Generated on: 2026-10-20
> Behavior change: none
> From backlog: BL-0011
> Depends on: SPEC-0013 (trash listing and recovery, implemented; the backlog calls it SPEC-0011, its pre renumbering id)
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt
### DBT-001: rename and `trash_entries` insert commit separately
- **Current structure:** `delete_path_at` renames via `rename_with_collision_retry` (`crates/core/src/core/fs_ops.rs:1264`), then calls `record_trash_entry` (`fs_ops.rs:1268`). `purge::sweep_project_files` does the same (`crates/core/src/purge.rs:89`, `:95`). `MetaBackend::rename` commits its own transaction (`crates/core/src/storage/meta.rs:1984`, `run_retrying`), and `record_trash_entry` opens another (`meta.rs:616`); its doc comment (`meta.rs:607-615`) admits the joint commit is missing.
- **Cost paid today:** a crash between the two commits leaves a file live in the trash directory with no `trash_entries` row until the lazy backfill (`meta.rs:581`) invents a best effort `deleted_at`. The purge sweep carries a compensating rename back (`purge.rs:98-106`) that exists only because the two steps are not one unit. SPEC-0013 FR-NEW-002 and NFR 7.4 ("a failed rename never leaves an orphaned row, a failed insert never leaves an untracked rename") are met only by compensation, not by construction.
- **Target structure:** one `MetaBackend` method performs the rename and the row insert in one transaction, implemented once in the generic relational impl; `rename_with_collision_retry` calls it; the sequential calls and the purge rename back are deleted.
- **Observable behavior:** unchanged for every input the suite covers; the crash window closes.

## 2. Perimeter: the Current State
| File | Change | Occurrences (recounted with `rg -n "record_trash_entry|rename_with_collision_retry" crates`, doc comments excluded) |
|---|---|---|
| `crates/core/src/storage/traits.rs` | add `TrashEntry` struct and trait method `rename_into_trash` after `rename` (`:209`) | 0 existing, 2 additions |
| `crates/core/src/storage/meta.rs` | factor the body of `rename` (`:1984-2037`) into `tx_rename`; add `rename_into_trash` in `impl MetaBackend for RelationalMetaStore` (`:1644`); `record_trash_entry` (`:616`) becomes `#[cfg(test)]`; fix its doc comment (`:607-615`) | 1 def |
| `crates/core/src/storage/volume.rs` | add `VolumeClient::rename_into_trash` next to `rename` (`:208`) | 0 existing |
| `crates/core/src/core/fs_ops.rs` | doc comment `:3160` updated; `rename_with_collision_retry` (`:1171`) gains a `trash: Option<&TrashEntry>` parameter; `delete_path_at` (`:1264-1279`) drops the `record_trash_entry` call (`:1268`); test callers `:3306`, `:3393`, `:3409` add `None`; test fixture `:3447` unchanged (stays valid, `cfg(test)`); test double `ScriptedRenameMeta` (`:3170`) implements the new method | 1 def, 1 prod caller, 1 prod record call, 3 test callers, 1 test fixture |
| `crates/core/src/purge.rs` | `sweep_project_files` (`:89-107`): pass the entry to the retry fn, delete the `record_trash_entry` call (`:95`) and the rename back; `use` at `:10` stays; doubles `AlwaysCollideMeta` (`:937`) and `FailingRenameMeta` (`:1578`) implement the new method; test 445 (`:1068`) rewritten | 1 import, 1 call, 1 record call |
| `crates/core/src/storage/conformance.rs` | fixture callers `:1105-1106` unchanged; add conformance case for `rename_into_trash` | 2 |
| `crates/core/src/tools/trash.rs` | fixture caller `:419` unchanged | 1 |
| `AGENTS.md` | line 78 sentence on `trash_entries` bookkeeping: add "written in the rename's own transaction" | 1 |
| `.agent_docs/architecture.md` | one line under the request lifecycle or safety section on the joint write (append, no restructure) | 1 |

**Non source occurrences:** none in `.env.example`, CI config, `Makefile`, container files, manifests or scripts (`rg -l "record_trash_entry|rename_with_collision_retry"` outside `crates/`, `sdlc/` returns nothing). Specs naming the symbols are history and are not edited: `sdlc/specs/SPEC-0013-trash-listing-and-recovery/{design,spec}.md`, `sdlc/specs/SPEC-0025-deleted-files-reenter-search/sdlc-spec.md`.
**Contracts touched:** `MetaBackend` (internal: 1 production impl at `meta.rs:1644`, 3 test doubles at `purge.rs:937`, `purge.rs:1578`, `fs_ops.rs:3170`); `rename_with_collision_retry` (`pub(crate)`, internal); no MCP tool, REST route, config key or schema is touched.
**Guardrail coverage:** the trash suite is E2E-NEW-401 to 458 in `fs_ops.rs`/`purge.rs`, plus `tools/trash.rs` tests and `conformance.rs`. **Test command:** `cargo test --workspace`; opt in engines: `docker compose -f docker-compose.test.yml up -d` then the suites in `.agent_docs/testing.md`.
**Total:** 8 files edited (`traits.rs`, `meta.rs`, `volume.rs`, `fs_ops.rs`, `purge.rs`, `conformance.rs`, `AGENTS.md`, `.agent_docs/architecture.md`); `tools/trash.rs` is read only. 14 code occurrences of `record_trash_entry|rename_with_collision_retry`: `purge.rs` 3 (`:10`, `:89`, `:95`), `tools/trash.rs` 1 (`:419`), `fs_ops.rs` 7 (`:1171`, `:1264`, `:1268`, `:3306`, `:3393`, `:3409`, `:3447`), `meta.rs` 1 (`:616`), `conformance.rs` 2 (`:1105`, `:1106`). 4 doc comment mentions (`purge.rs:930`, `fs_ops.rs:1258`, `fs_ops.rs:3160`, `tools/trash.rs:232`): `:1258`, `:930` and `:232` stay valid because the function name is unchanged; `fs_ops.rs:3160` is updated to say the double intercepts `rename_into_trash` too. Recounted by `rg` on 2026-10-20.

## 3. Class sweep
n/a, not a security entry.

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-U]: A joint rename and trash entry operation
> The metadata store SHALL expose `MetaBackend::rename_into_trash(&self, src: &str, dst: &str, entry: &TrashEntry) -> Result<()>` that renames `src` to `dst` and inserts the `trash_entries` row for `dst` in ONE relational transaction, implemented once in the relational `impl` and not per dialect.
- **Old form:** `rename` then `record_trash_entry`, two commits.
- **New form:** `TrashEntry { original_path: String, size: i64, kind: String, deleted_at: String, deleted_by: Option<String> }` in `storage/traits.rs`; the row's `trash_path` is `dst` of the attempt that succeeds. The method shares one `tx_rename(tx, dialect, volume, src, dst)` helper with `rename`, so the move logic exists once.
- **Applies to:** `traits.rs`, `meta.rs`, `volume.rs`. `VolumeClient::rename_into_trash(src, dst, entry)` SHALL delegate to `self.meta.rename_into_trash`, never to `self.trash`; `self.trash` is only the presence flag that decides whether `Some(&TrashEntry)` is passed.
- **Casing convention:** Rust snake_case for functions, CamelCase for the struct.

#### DR-002 [EARS-U]: Callers stop doing the two step sequence
> `rename_with_collision_retry` SHALL take `trash: Option<&TrashEntry>`; WHEN it is `Some` each attempt (`dst`, `dst~1` ... `dst~50`) SHALL call `VolumeClient::rename_into_trash`, otherwise `VolumeClient::rename`; `delete_path_at` and `sweep_project_files` SHALL pass `Some` exactly when `client.trash.is_some()` and SHALL NOT call `record_trash_entry`.
- **Old form:** `rename_with_collision_retry(client, src, dst)` plus a later `store.record_trash_entry(...)`.
- **New form:** `rename_with_collision_retry(client, src, dst, trash)`; the entry carries the values the callers computed before the move (`node.size`, `node.kind`, `now_iso()`, `Some(person)` or `None`).
- **Applies to:** `fs_ops.rs`, `purge.rs`.
- **Casing convention:** unchanged.

#### DR-003 [EARS-U]: The compensating rename back is deleted
> `sweep_project_files` SHALL NOT issue a `rename` back to the original path after a failed trash entry insert; the transaction rollback leaves the file at its original path.
- **Old form:** `purge.rs:98-106`. **New form:** a failed joint call returns `Err`, the existing per file error isolation (`Err(_) => continue`, `purge.rs:89`) applies.

#### DR-004 [EARS-U]: `record_trash_entry` is test only
> `record_trash_entry` SHALL be compiled only under `#[cfg(test)]` (fixture use at `conformance.rs:1105`, `tools/trash.rs:419`, `fs_ops.rs:3447`), so no production code can reintroduce the two step write (DEC-003).

### 4.2 Invariance (MANDATORY)
#### DR-005 [EARS-U]: Behavior invariance
> The system SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change: the same `fs.delete` response, the same `trash_entries` rows (`trash_path`, `original_path`, `size`, `kind`, `deleted_at`, `deleted_by`), the same collision suffixes, the same error codes.

#### DR-006 [EARS-O]: Collision semantics preserved
> IF the destination is occupied THEN the transaction of that attempt SHALL roll back with `ERR_NO_CLOBBER`, leaving no node and no row at that `dst`, and `rename_with_collision_retry` SHALL retry at the next suffix up to `~50`; any other error SHALL propagate with no retry; exhaustion SHALL return `ERR_INTERNAL_ERROR` with `src` untouched and zero rows (FR-NEW-004/005/006 of SPEC-0013).

### 4.3 Atomicity
#### DR-007 [EARS-U]: All or nothing
> The system SHALL make a failure of the row insert undo the rename, and a failure of the rename (including `ensure_storable_path` on a descendant, `meta.rs` rename body) leave no row, within one transaction.

### 4.4 Compatibility
#### DR-008 [EARS-UB]: No contract change
> The system SHALL NOT change any MCP tool schema, REST payload, `trash_entries` column, or `TOOL_CONTRACT.txt`/`tool-contract-golden.json` content.
- The lazy backfill (`backfill_trash_entry`, `meta.rs:581`) stays unchanged for legacy untracked nodes.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `MetaBackend` trait | internal | no | add a required method (no default, so a store cannot silently skip the row); 3 test doubles updated | n/a | n/a |
| `rename_with_collision_retry` signature | internal | no | add parameter, update 3 test callers | n/a | n/a |
| `record_trash_entry` | internal | no | `#[cfg(test)]` | n/a | n/a |

**Dual maintenance semantics:** none; no public contract moves.

## 6. Decisions
- **DEC-001:** New trait method rather than letting `VolumeClient` open a transaction. **Alternatives considered:** (a) `VolumeClient` begins a dialect transaction: rejected, a store never speaks a driver (AGENTS.md convention) and `meta` is an `Arc<dyn MetaBackend>`; (b) change `MetaBackend::rename` to always take an optional entry: rejected, widens every caller and test double for a rare path. **Rationale:** the relational impl already owns both statements. **Implemented by:** DR-001. **Evidence:** `meta.rs:1644`, `volume.rs:208`.
- **DEC-002:** Entry passed only when `client.trash.is_some()`. **Alternatives considered:** always write the row: rejected, it would change behavior for clients built without a trash store (tests, E2E-NEW-457 style fixtures). **Implemented by:** DR-002. **Evidence:** `fs_ops.rs:1266`, `purge.rs:94`.
- **DEC-003:** Keep `record_trash_entry` as `#[cfg(test)]` instead of deleting it. **Alternatives considered:** delete and rewrite 4 fixture callers: rejected, churn with no behavior gain. **Implemented by:** DR-004.
- **DEC-004:** Required trait method, no default. **Alternatives considered:** default delegating to `rename` and ignoring the entry: rejected, a silent no row path is the very bug being fixed. **Implemented by:** DR-001.
- **Assumption:** SQLite, PostgreSQL and SQL Server all roll back DML on a returned `Err` from the `run_retrying` closure, as the existing multi row `rename` already relies on (`meta.rs` comment "The whole rename is one transaction").

## 7. Declared breaks
None. Behavior change is none; the fix narrows a crash window only.

## 8. Tests
### 8.1 Non regression: the success criterion
The existing test suite passes unmodified except the rows of 8.4:
    cargo test --workspace && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all -- --check
and, with the opt in engines up (`docker compose -f docker-compose.test.yml up -d`), the PostgreSQL and SQL Server suites of `.agent_docs/testing.md`.

### 8.2 Compatibility tests
None: no public contract is dual maintained (Section 5).

### 8.3 Structural tests
#### DT-001: no sequential write in production code
- **Asserts:** DR-002, DR-004. No `record_trash_entry` call outside `#[cfg(test)]` code.
- **Method:** a test that reads `fs_ops.rs`, `purge.rs` sources and asserts the production region (before `#[cfg(test)] mod tests`) does not contain `record_trash_entry`; plus compile time proof (`cargo build --release` has no caller).
#### DT-002: single transaction per soft delete
- **Asserts:** DR-001. Location: `fs_ops.rs` tests. A counting `RelationalDb` wraps `meta`'s db; the counter is reset just before `VolumeClient::rename_into_trash` and read right after. It expects exactly 1 `begin()` and asserts that the `UPDATE nodes` and the `INSERT INTO trash_entries` ran inside it.
- **Method:** counting wrapper over `SqliteRelationalDb::open_in_memory`.

### 8.3.1 Behavior and atomicity tests (the `DT-` guardrails the old suite cannot see)
#### DT-003: insert failure undoes the rename
- **Validates:** DR-001, DR-007. Location: `purge.rs` tests next to `FailTrashInsertOnceDb` (`purge.rs:1016`). Given a file `/a.txt` and a db that fails `INSERT INTO trash_entries` (that same type) wrapped around the store used as `meta`. When `rename_into_trash("/a.txt", "/.mcp_trash/x", entry)` runs. Then `Err`, `/a.txt` exists, `/.mcp_trash/x` does not, `count_trash_entries_for_test` is 0.
#### DT-004: rename failure leaves no row
- **Validates:** DR-007. Fault injected through the counting `RelationalDb` wrapper (as DT-002): `UPDATE nodes` fails on the second descendant of `/d`, which holds 3 files. Then `rename_into_trash` returns `Err`, no path moved, 0 rows. The `ensure_storable_path` ceiling variant (`meta.rs:141-154`) runs only in the SQL Server opt in suite, since it is a no-op on SQLite and PostgreSQL.
#### DT-005: success is visible together
- **Validates:** DR-001. After a successful call, a second reader sees the node at `dst`, none at `src`, and the row `trash_path = dst`, `original_path = "/a.txt"`.
#### DT-006: directory move
- **Validates:** DR-001, DR-005. `/d` with 3 files: all four paths move, exactly one row, `kind = "dir"`.
#### DT-007: collision rolls back then retries
- **Validates:** DR-006. `dst` occupied: attempt one returns `ERR_NO_CLOBBER` with no row; `fs.delete` ends at `dst~1` with one row keyed `dst~1` (extends E2E-NEW-404).
#### DT-008: exhaustion
- **Validates:** DR-006. 51 occupied slots (as E2E-NEW-405): `ERR_INTERNAL_ERROR`, `/x` live, 0 rows.
#### DT-009: non collision error aborts the retry loop
- **Validates:** DR-006. A scripted `ERR_INTERNAL_ERROR` on attempt two, with an entry present: no third attempt, 0 rows.
#### DT-010: purge atomicity
- **Validates:** DR-003, DR-007. Replaces E2E-NEW-445's mechanism: the failing db backs `meta` itself; the sweep returns 0, the file is live with its original `rev`, 0 rows, and no node exists under the trash directory (proof that no rename back was needed).
#### DT-011: purge row shape
- **Validates:** DR-005. Sweep of a stale file writes a row with `deleted_by` NULL and the same shape as E2E-NEW-403/440.
#### DT-012: legacy backfill unchanged
- **Validates:** DR-008. A trash node with no row is still picked up by `trash_list` (existing E2E-NEW-4xx backfill tests must stay green; one explicit assertion added).
#### DT-013: conformance
- **Validates:** DR-001 on every engine. A case in `storage/conformance.rs` calling `rename_into_trash` on the shared suite (SQLite always; PostgreSQL and SQL Server through the opt in suites). It also asserts rollback on every engine: `rename_into_trash` onto an occupied `dst` returns `ERR_NO_CLOBBER`, `src` is still present, `dst` is unchanged and `trash_entries` has no row for `dst`.

### 8.4 Existing tests to modify (the "rows of 8.4" in 8.1)
| Test file | Test | Change | Justification |
|---|---|---|---|
| `crates/core/src/core/fs_ops.rs` | E2E-NEW-441, 449, 450 (`:3306`, `:3393`, `:3409`) | add `None` argument | signature change, assertions untouched |
| `crates/core/src/purge.rs` | E2E-NEW-445 (`:1068`) | fault injected in the db behind `meta`, not behind a second store | the second store no longer sits on the rename path; asserted outcome (file live, 0 rows, purged 0) unchanged |
| `crates/core/src/{purge.rs:937,:1578; core/fs_ops.rs:3170}` | doubles `AlwaysCollideMeta`, `FailingRenameMeta`, `ScriptedRenameMeta` | implement `rename_into_trash`: apply the same interception as their `rename`, else delegate to `inner.rename_into_trash` | required trait method; keeps E2E-NEW-441/443/444/447/449/450 meaningful on the joint path |

### 8.5 Security tests
n/a.

## 9. Implementation order
1. Add `TrashEntry` and the required `rename_into_trash` to the trait; factor `tx_rename` in `meta.rs`; implement the method; update the 3 doubles. Green: `cargo test --workspace` (no caller yet). Add DT-003 to DT-006, DT-013.
2. Add `VolumeClient::rename_into_trash`; extend `rename_with_collision_retry` and its 3 test callers. Green. Add DT-007 to DT-009.
3. Migrate `delete_path_at` and `sweep_project_files`; delete the rename back; rewrite test 445 as DT-010; add DT-002, DT-011, DT-012.
4. Make `record_trash_entry` `#[cfg(test)]`, fix its doc comment; add DT-001. Update `AGENTS.md` line 78 and `.agent_docs/architecture.md`. Full gate green.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 1 | 3 | NOT-IMPLEMENTABLE |
| 2 | 0 | 5 | IMPLEMENTABLE |

**Amendments applied:** round 1: perimeter total recounted (14 code occurrences, 8 files edited), `VolumeClient` delegates to `meta`, DT-002 and DT-013 sharpened; round 2: doc comment count 4, DT-003 location, DT-004 fault injection, DT-010 proof, heading numbering.
