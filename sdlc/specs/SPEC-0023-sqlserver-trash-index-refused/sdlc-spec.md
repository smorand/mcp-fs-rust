# SQL Server metadata store cannot be created: Bug Specification

> Generated on: 2026-10-12
> Id: SPEC-0023
> Nature: BUG
> Depth: S
> Depth evidence: one module (metadata store schema), one defective behavior, 4 requirements, no public contract change, no persisted data change (SQL Server never kept the table, see Rollback posture)
> Status: designed
> Verdict: authority exists
> Defect class: n/a (code violates a clear authority)
> Severity: High
> From backlog: BL-0046
> Security: n/a
> CVSS: n/a
> Affected: every deployment whose `infra.meta` store is SQL Server, since `0bfb359` (2026-10-06)
> Fixed in: unreleased

## 1. Defect

### BUG-001: the SQL Server metadata schema fails to apply, so the store never opens
- **Observed behavior:** opening a metadata store on SQL Server fails with
  `ERR_INTERNAL_ERROR` `sqlserver: Column 'deleted_at' in table 'trash_entries' is of a type that is invalid for use as a key column in an index. [1919]`.
  Reproduced on 2026-10-12 against the compose SQL Server:
  `MCPFS_TEST_MSSQL_DSN=... cargo test -p mcp-fs-core --all-features --lib sqlserver_passes_the_conformance_suite`
  panics at `crates/core/src/storage/conformance.rs:1110` with that message, `0 passed; 1 failed`.
- **Expected per authority:**
  - SPEC-0013/FR-NEW-001: the `trash_entries` table is "implemented across sqlite/postgres/sqlserver
    dialects" (`sdlc/specs/SPEC-0013-trash-listing-and-recovery/spec.md:85`), with "a secondary
    `(volume_id, deleted_at)` index supporting the required ordering" (`:111`).
  - "a text column that participates in a primary key or an index must carry a length"
    (`crates/core/src/storage/rel/dialect.rs:21-22`; `.agent_docs/backends.md:89`; `AGENTS.md:215`).
  - "sqlserver must satisfy exactly the same cases as sqlite" (`conformance.rs:1110`).
- **Reproduction:** server configured with `infra.meta` on SQL Server, a fresh database. Any `fs.*` tool
  on a project, for example `fs.write_bytes {mount_id: "atlas", path: "/a.txt", ...}`. Expected: the
  write succeeds. Actual: the metadata store cannot be opened, so the call fails with the 1919 message.
- **Root cause:** `crates/core/src/storage/meta.rs:207` declares `deleted_at` as unbounded text
  (`ColumnType::Text`), rendered `NVARCHAR(MAX)` on SQL Server (`dialect.rs:233`), and
  `meta.rs:304-309` indexes it (`idx_trash_entries_deleted_at` on `volume_id, deleted_at`). SQL Server
  refuses an `NVARCHAR(MAX)` index key.
- **Contributing factors:** the SQL Server suite is opt in (`conformance.rs:1100-1107`) and not part of
  `make check`; nothing checks that every indexed text column is bounded.
- **Impact:** SQL Server metadata stores have not been usable since 2026-10-06: every file operation on
  them fails. SQLite and PostgreSQL are unaffected (`TEXT` is indexable, `dialect.rs:218`, `:226`).
  There is no workaround except switching engine.
- **Rollback posture:** SQL Server applies the schema in one transaction and rolls it back on any error
  (`crates/core/src/storage/rel/sqlserver.rs:455-467`), so no SQL Server database holds a
  `trash_entries` table today. SQLite and PostgreSQL databases already hold one; their stored values
  and type rendering must stay readable by both the fixed and the previous code.

## 2. Expected Behavior

#### BR-001 [EARS-E]: The SQL Server metadata schema applies
> WHEN a metadata store is opened on a SQL Server database THE Storage context SHALL apply its schema
> without error, and applying it again SHALL change nothing.
- **Source:** restored from SPEC-0013/FR-NEW-001 and `dialect.rs:21-22`.

#### BR-002 [EARS-E]: Trash listing on SQL Server
> WHEN `fs.trash_list` is called on a SQL Server metadata store THE Storage context SHALL return the
> entries ordered by `deleted_at` descending, each `deleted_at` the RFC 3339 value recorded at deletion.
- **Source:** restored from SPEC-0013/FR-NEW-007 (`spec.md:91`).
- **Exact names:** `fs.trash_list`, response `{"entries": [...], "total": N}`, entry field `deleted_at`.

#### BR-003 [EARS-U]: Same cases as SQLite on SQL Server
> The Storage context SHALL pass, on SQL Server, every case of the conformance suite that it passes on SQLite.
- **Source:** restored from `conformance.rs:1110`.

#### BR-004 [EARS-UB]: Existing stores unchanged
> The Storage context SHALL NOT change the value or the listing order of any `deleted_at` already
> stored by a SQLite or PostgreSQL metadata store.
- **Source:** restored from SPEC-0013/FR-NEW-001 (`deleted_at (RFC3339)`), and the rollback posture above.

## 3. Edge Cases

| Case | Expected behavior | Requirement |
|------|-------------------|-------------|
| schema applied twice on SQL Server | second apply is a no op | BR-001 |
| longest `deleted_at` written today (`util::now_iso`, microseconds and offset, 32 characters: `2026-10-12T22:11:27.123456+00:00`, `crates/core/src/util/mod.rs:23-25`) | stored and listed unchanged on SQL Server | BR-002 |
| legacy trash node backfilled from its epoch prefix (`crates/core/src/tools/trash.rs:155-167`) | its reconstructed `deleted_at` stored and listed unchanged on SQL Server | BR-002 |
| SQLite and PostgreSQL databases created before the fix | open, list and order exactly as before | BR-004 |

## 4. Tests

### 4.1 End to end reproduction (mandatory)
#### BT-001: A file deleted on a SQL Server store is listed in the trash
- **Reproduces:** BUG-001
- **Driver:** `fs.write_bytes`, `fs.delete`, `fs.trash_list` tool calls on a server whose metadata store
  is SQL Server (`MCPFS_TEST_MSSQL_DSN`), skipped when unset.
- **Fails today because:** the store cannot open, error 1919 (`meta.rs:207`, `:304-309`).
- Given project `atlas` with a fresh SQL Server metadata database
- When the owner writes `/a.txt` (3 bytes), then `/b.txt`, deletes `/a.txt`, then deletes `/b.txt`
- Then each call succeeds, and `fs.trash_list {mount_id: "atlas"}` returns `total: 2` with entries whose
  `original_path` are `/b.txt` then `/a.txt` (newest deletion first).

#### BT-002: The conformance suite passes on SQL Server
- **Reproduces:** BUG-001 (the BL-0046 symptom)
- **Driver:** `sqlserver_passes_the_conformance_suite` (`conformance.rs:1099`), `MCPFS_TEST_MSSQL_DSN` set.
- **Fails today because:** 1919 during schema setup.
- Then every case passes, the SPEC-0022 null value case included.

### 4.2 New behavior
#### BT-010: Schema applied twice on SQL Server
- **Validates:** BR-001
- Given a SQL Server database where the metadata schema was applied / When it is applied again / Then no error, and the table set and index set are unchanged.

#### BT-011: Recorded timestamp returned verbatim on SQL Server
- **Validates:** BR-002
- Given an entry recorded with `deleted_at` `2026-10-12T22:11:27.123456+00:00` and one with `2026-10-11T08:00:00.000000+00:00`
- When `fs.trash_list` runs / Then `deleted_at` values come back byte for byte, the 2026-10-12 entry first.

### 4.3 Non regression
#### BT-020: SQLite and PostgreSQL trash listing unchanged
- **Protects:** SPEC-0013/FR-NEW-007 on the default engine and on PostgreSQL
- The existing SQLite suites (`make check`) and `postgres_passes_the_conformance_suite` stay green.

#### BT-021: A PostgreSQL database created before the fix opens and lists unchanged
- **Protects:** BR-004
- Given a PostgreSQL schema created by the current code with two trash entries / When the fixed code opens it and lists the trash / Then no migration error, and the same two entries in the same order with the same `deleted_at`.

### 4.4 Existing tests asserting the current behavior
| Test file | Test name | Action | Reason |
|-----------|-----------|--------|--------|
| `crates/core/src/storage/conformance.rs:1099` | `sqlserver_passes_the_conformance_suite` | Kept | it is BT-002; red today |
| `crates/core/src/tools/trash.rs` (test module) | trash listing tests | Kept | SQLite reference behavior |
| `crates/core/src/trash_screen.rs:542-650` | `/app/trash` tests | Kept | unaffected |

No existing test asserts the failure.

## 5. Existing Specs Affected
None deviated. Restores SPEC-0013/FR-NEW-001 and FR-NEW-007 on SQL Server.

## 6. Assumptions
- **Assumption:** no other index or key in any schema of the tree sits on an unbounded text column. The
  three metadata indexes were read (`meta.rs:297-315`); BR-003 catches any other on the full SQL Server suite.
- **Backlog entries created by this run:** none.

## 7. Implementability Checklist

| # | Item | Result |
|---|---|---|
| 1 | No technical decision | PASS (root cause only; the fix type is left to design) |
| 2 | No buried requirement | PASS |
| 3 | Observable names spelled | PASS |
| 4 | No forced choice | PASS (order, value, idempotency stated) |
| 5 | Every test specified | PASS |
| 6 | Reproduction end to end | PASS (BT-001 tool calls, user visible) |
| 7ante | Security misclassification | PASS |
| 8 | No hidden prerequisite | PASS (`MCPFS_TEST_MSSQL_DSN` stated) |
| 9 | EARS clean | PASS |
| 10 | Right tool | PASS (4 requirements at S) |
| 11 | Length | PASS |
| 12 | Claims cited | PASS |
| 13 | Writer and reader read | PASS (writer `util/mod.rs:23-25`, `trash.rs:155-167`; reader `meta.rs:1469-1470`, `trash.rs:122-125`) |
| 14 | Verdict consistent | PASS |

| Verdict | IMPLEMENTABLE |
|---|---|
| Open F failures | 0 |
| A failures | amended in place: 0; moved to backlog: none |
