# PostgreSQL refuses absent values in numeric columns: Bug Specification

> Generated on: 2026-10-12
> Id: SPEC-0022
> Nature: BUG
> Depth: S
> Depth evidence: one root cause line in one module (relational layer, PostgreSQL engine), 6 requirements, no public contract change, no data model change; 3 tool families expose the same defect
> Status: specified
> Verdict: authority exists
> Defect class: n/a (code violates a clear authority)
> Severity: High
> From backlog: BL-0042
> Security: n/a
> CVSS: n/a
> Affected: every deployment with a PostgreSQL relational store; SPEC-0014 purge config and SPEC-0010 quota clearing since they shipped, SPEC-0019 conversions since `a9caa0f`
> Fixed in: unreleased

## 1. Defect

### BUG-001: on PostgreSQL, writing an absent (null) value into a numeric column fails
- **Observed behavior:** the PostgreSQL engine binds every null parameter as a text value
  (`crates/core/src/storage/rel/postgres.rs:160`, `SqlValue::Null => args.add(None::<String>)`).
  PostgreSQL then refuses it for a `BIGINT` column with SQLSTATE `42804`, surfaced as
  `ERR_INTERNAL_ERROR` `postgres: column "<name>" is of type bigint but expression is of type text [42804]`
  (mapping at `postgres.rs:145-148`). Reproduced on 2026-10-12 against the compose PostgreSQL:
  `MCPFS_TEST_PG_DSN=... cargo test -p mcp-fs-core --features postgres --lib conformance` fails
  `postgres_passes_the_conformance_suite` (`crates/core/src/storage/conformance.rs:1045`) with exactly
  that message for `superseded_at`. A direct `psql` probe confirms `UPDATE t SET a=$1` with a text
  typed null into `a bigint` fails the same way.
- **Expected per authority:** "Three engines are supported, selected per store from the YAML config"
  (`.agent_docs/backends.md:3-6`); the conformance suite requires "postgres must satisfy exactly the
  same cases as sqlite" (`crates/core/src/storage/conformance.rs:1045`). Each affected tool documents
  a null value as valid input or output (cited per requirement below).
- **Reproduction:** server with a PostgreSQL relational store, admin Ada, project `Atlas`.
  Ada calls `admin.set_project_quota {project_id: "Atlas", max_mb: 5}` then
  `admin.set_project_quota {project_id: "Atlas", max_mb: null}`. Expected: `{project_id: "Atlas",
  max_mb: null}` and `admin.get_project_quota` returns `max_mb: null`. Actual: the second call fails with
  `ERR_INTERNAL_ERROR` naming `quota_bytes`, and the 5 MB limit stays.
- **Root cause:** `crates/core/src/storage/rel/postgres.rs:160` gives every null parameter the text
  type, whatever the column it is written to. SQLite stores an untyped null
  (`crates/core/src/storage/rel/sqlite.rs:52`).
- **Production sites hit (recounted):** 7 null binds into `BIGINT` columns across 5 statements:
  - `storage/admin.rs:239-240`: project creation with a purge config, columns `file_retention_days`,
    `project_retention_days` (`admin.rs:68-69`).
  - `storage/admin.rs:483-484`: purge config update, same columns.
  - `storage/admin.rs:494-495`: quota update, column `quota_bytes` (`admin.rs:102-106`).
  - `storage/meta.rs:712`: artifact set insert, `superseded_at` (`meta.rs:256`).
  - `storage/meta.rs:747` and `:753`: image insert, `page` and `caption_line` (`meta.rs:268`, `:274`).
  Null binds into text columns are accepted and unaffected (`git/oauth/persistence.rs:147`,
  `storage/meta.rs:487`, `:495`, `:1257`).
- **Contributing factors:** the conformance suite never sets a quota to null or a retention to null
  (`rg -n "set_quota\(|set_purge_config\(" crates/core/src/storage/conformance.rs`: 0 hits); the
  PostgreSQL suite is opt in (`conformance.rs:1035-1038`) and is not part of `make check`, so the SPEC-0019
  case failing went unnoticed until US-0015.
- **Impact:** on PostgreSQL: every document conversion fails (`fs.documentize`, `fs.write_bytes` with
  `trigger_documentation_service`, REST upload with the same flag, all through
  `crates/core/src/docs/artifacts/capture.rs:206`); a quota cannot be cleared; a purge config with a
  disabled axis cannot be saved or given at project creation. SQLite deployments are unaffected. No
  workaround short of switching engine.
- **Rollback posture:** reverting the fix restores today's failures; no stored row changes shape, and
  rows written after the fix read back identically with the old code.

## 2. Expected Behavior

All requirements apply WHILE the relational store is PostgreSQL. Values are the snake_case tool
fields of `TOOL_CONTRACT.txt`.

#### BR-001 [EARS-E]: Clearing the quota
> WHEN a platform admin calls `admin.set_project_quota` with `max_mb` null or absent THE Storage context
> SHALL return `{"project_id": <id>, "max_mb": null}` and `admin.get_project_quota` SHALL then report `max_mb: null`.
- **Source:** restored from `TOOL_CONTRACT.txt:62` ("max_mb absent or null clears the quota (unlimited)").

#### BR-002 [EARS-E]: Purge config with a disabled axis
> WHEN `admin.set_purge_config` is called with `file_retention_days` or `project_retention_days` null
> THE Storage context SHALL save the config and report that axis as null when the config is read back.
- **Source:** restored from `TOOL_CONTRACT.txt:67` ("Either retention value may be absent to disable that axis").

#### BR-003 [EARS-E]: Project creation with a disabled axis
> WHEN `admin.create_project` is called with `autopurge_enabled: true` and one retention value null
> THE Storage context SHALL create the project with that axis null.
- **Source:** restored from `TOOL_CONTRACT.txt:8` (both retention params default to null).

#### BR-004 [EARS-E]: Conversions keep their artifacts
> WHEN a document conversion completes THE Storage context SHALL keep its artifact set, exactly as on SQLite.
- **Source:** restored from SPEC-0019/FR-NEW-001 (`sdlc/specs/SPEC-0019-document-artifacts/sdlc-spec.md:201`)
  and the conformance parity rule (`conformance.rs:1045`).

#### BR-005 [EARS-E]: An image with no page
> WHEN a conversion yields an image with no page THE Storage context SHALL report it with `"page": null` in `fs.list_images`.
- **Source:** restored from SPEC-0019/FR-NEW-003 (`sdlc-spec.md:233`); SQLite behavior asserted by
  `spec_0019_e2e_115_a_word_image_has_an_empty_page` (`crates/core/src/tools/artifacts.rs:1770`).

#### BR-006 [EARS-UB]: No engine specific refusal of absent values
> The Storage context SHALL NOT refuse, on any supported engine, a null value written to a nullable
> column that the same operation accepts on SQLite.
- **Source:** restored from `.agent_docs/backends.md:3-6` and `conformance.rs:1045`.

## 3. Edge Cases

| Case | Expected behavior | Requirement |
|------|-------------------|-------------|
| quota cleared when already unlimited | `max_mb: null`, no error | BR-001 |
| both retention axes null | both read back null | BR-002 |
| null into a text column (token expiry, deleted_by) | unchanged, accepted | BR-006, BT-020 |
| SQLite and SQL Server | unchanged results for every case above | BT-021, BT-022 |

## 4. Tests

The PostgreSQL tests run when `MCPFS_TEST_PG_DSN` is set, as the existing PostgreSQL suites do
(`conformance.rs:1035-1038`); `make test-e2e-full` sets it (`.agent_docs/testing.md:225`).

### 4.1 End to end reproduction (mandatory)
#### BT-001: Clearing a quota on PostgreSQL
- **Reproduces:** BUG-001
- **Driver:** the `admin.set_project_quota` and `admin.get_project_quota` tool calls on a server whose admin store is PostgreSQL.
- **Fails today because:** the null for `quota_bytes` is bound as text (`postgres.rs:160`).
- Given project `Atlas` and Ada's call `admin.set_project_quota {project_id: "Atlas", max_mb: 5}` succeeded
- When Ada calls `admin.set_project_quota {project_id: "Atlas", max_mb: null}`
- Then it returns `{"project_id": "Atlas", "max_mb": null}`
- And `admin.get_project_quota {project_id: "Atlas"}` returns `max_mb: null`

#### BT-002: The conformance suite passes on PostgreSQL
- **Reproduces:** BUG-001 (the BL-0042 symptom)
- **Driver:** `postgres_passes_the_conformance_suite` (`conformance.rs:1033`)
- **Fails today because:** `superseded_at` null bound as text.
- Given `MCPFS_TEST_PG_DSN` set / When the suite runs / Then every case passes, the artifacts case included.

### 4.2 New behavior
#### BT-010: Purge config with null retention axes
- **Validates:** BR-002
- Given `Atlas` on PostgreSQL / When `admin.set_purge_config {project_id: "Atlas", autopurge_enabled: true, use_internal_purge: true, file_retention_days: 30, project_retention_days: null}`, then the same with `file_retention_days: null, project_retention_days: null`
- Then both calls succeed, and the config read back has `project_retention_days` null after the first and both axes null after the second.

#### BT-011: Project creation with a null axis
- **Validates:** BR-003
- When `admin.create_project {project_id: "Borea", owner: "olivia@example.com", autopurge_enabled: true, file_retention_days: 7, project_retention_days: null}` on PostgreSQL
- Then the project exists and its config reads `file_retention_days: 7`, `project_retention_days: null`.

#### BT-012: A conversion on PostgreSQL keeps its artifacts
- **Validates:** BR-004, BR-005
- Given a PostgreSQL meta store and the stubbed conversion of `spec_0019_e2e_115` (`artifacts.rs:1770`): one image `Org chart` with no page
- When `fs.documentize` runs on `/memo.docx`
- Then it succeeds, and `fs.list_images` returns `{"images": [{"id": "image-1", "caption": "Org chart", "page": null}], "marker": null}`.

#### BT-013: Reconverting supersedes the previous set
- **Validates:** BR-004
- Given BT-012 done / When `/memo.docx` is rewritten and converted again / Then `fs.list_images` reports the new conversion's images only.

### 4.3 Non regression
#### BT-020: Null into a text column still accepted on PostgreSQL
- **Protects:** `live_migrate_is_idempotent_and_round_trips_every_value_type` (`postgres.rs:428`), which inserts a null into the text column `t`
- Then it still reads back null.

#### BT-021: SQLite results unchanged
- **Protects:** every SQLite case of BT-001, BT-010 to BT-013 (the existing SQLite suites, `make check`)
- Then `make check` green with the same counts plus the new tests.

#### BT-022: SQL Server conformance unchanged
- **Protects:** the SQL Server conformance case, run when `MCPFS_TEST_MSSQL_DSN` is set
- Then it passes. ASSUMED today: SQL Server accepts its null binding (`crates/core/src/storage/rel/sqlserver.rs:204`) through implicit conversion; not run in this analysis.

### 4.4 Existing tests asserting the current behavior
| Test file | Test name | Action | Reason |
|-----------|-----------|--------|--------|
| `crates/core/src/storage/conformance.rs:1033` | `postgres_passes_the_conformance_suite` | Kept | it is BT-002; red today |
| `crates/core/src/storage/rel/postgres.rs:428` | `live_migrate_is_idempotent_and_round_trips_every_value_type` | Kept | BT-020 |
| `crates/core/src/tools/artifacts.rs:1770` | `spec_0019_e2e_115_a_word_image_has_an_empty_page` | Kept | SQLite reference for BT-012 |

No existing test asserts the failure.

## 5. Existing Specs Affected
None deviated. Restores SPEC-0010 (quota clearing), SPEC-0014 (purge config axes) and SPEC-0019
(FR-NEW-001, FR-NEW-003) on PostgreSQL.

## 6. Assumptions
- **Assumption:** SQL Server is not affected (BT-022); if its suite fails, that is a separate defect.
- **Assumption:** the 7 sites listed are all production null binds into non text columns, from a full
  sweep of `.bind(` sites on 2026-10-12; BR-006 covers any site missed.
- **Backlog entries created by this run:** none.

## 7. Implementability Checklist

| # | Item | Result |
|---|---|---|
| 1 | No technical decision | PASS |
| 2 | No buried requirement | PASS |
| 3 | Observable names spelled | PASS (tool names, fields, error code and message) |
| 4 | No forced choice | PASS |
| 5 | Every test specified | PASS |
| 6 | Reproduction end to end | PASS (BT-001 through tool calls, user visible) |
| 7ante | Security misclassification | PASS (no bypass, injection or exposure) |
| 8 | No hidden prerequisite | PASS (`MCPFS_TEST_PG_DSN` stated) |
| 9 | EARS clean | PASS |
| 10 | Right tool | PASS (6 requirements at S) |
| 11 | Length | PASS |
| 12 | Claims cited | PASS (SQL Server labeled ASSUMED) |
| 13 | Writer and reader read | PASS (writer `postgres.rs:160`, readers via the tools; SQLite `sqlite.rs:52` parity) |
| 14 | Verdict consistent | PASS |

| Verdict | IMPLEMENTABLE |
|---|---|
| Open F failures | 0 |
| A failures | amended in place: 0; moved to backlog: none |
