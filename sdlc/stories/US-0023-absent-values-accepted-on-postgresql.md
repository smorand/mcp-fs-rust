# US-0023: Absent values accepted on PostgreSQL

> Spec: SPEC-0022 (sdlc/specs/SPEC-0022-postgres-null-values-refused/)
> Nature: BUG
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 5
> Closes spec: yes

## Objective
On a PostgreSQL relational store, every null bound into a numeric column fails with
`ERR_INTERNAL_ERROR ... is of type bigint but expression is of type text [42804]`: document
conversions, clearing a quota, and purge configs with a disabled axis all fail. After this story they
behave exactly as on SQLite.

## Requirements covered
- SPEC-0022/BR-001: WHEN a platform admin calls `admin.set_project_quota` with `max_mb` null or absent THE Storage context SHALL return `{"project_id": <id>, "max_mb": null}` and `admin.get_project_quota` SHALL then report `max_mb: null`.
- SPEC-0022/BR-002: WHEN `admin.set_purge_config` is called with `file_retention_days` or `project_retention_days` null THE Storage context SHALL save the config and report that axis as null when the config is read back.
- SPEC-0022/BR-003: WHEN `admin.create_project` is called with `autopurge_enabled: true` and one retention value null THE Storage context SHALL create the project with that axis null.
- SPEC-0022/BR-004: WHEN a document conversion completes THE Storage context SHALL keep its artifact set, exactly as on SQLite.
- SPEC-0022/BR-005: WHEN a conversion yields an image with no page THE Storage context SHALL report it with `"page": null` in `fs.list_images`.
- SPEC-0022/BR-006: The Storage context SHALL NOT refuse, on any supported engine, a null value written to a nullable column that the same operation accepts on SQLite.
(All WHILE the relational store is PostgreSQL.)

## Independent Test
`docker compose -f docker-compose.test.yml up -d`, then
`MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' cargo test -p mcp-fs-core --all-features --lib -- conformance pg_null_e2e storage::rel::postgres`
all green; `make check` green. BT-001 reads the quota back through `admin.get_project_quota`.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0022): bind `SqlValue::Null` on PostgreSQL as an untyped parameter (OID 0) through a private `sqlx::Type`/`Encode` impl, at `crates/core/src/storage/rel/postgres.rs:160` (today `SqlValue::Null => args.add(None::<String>)`). `PgTypeInfo::with_oid(Oid(0))` exists in sqlx-postgres 0.9.0 (`type_info.rs:336`); `Encode` returns `IsNull::Yes`.
- DEC-002 (SPEC-0022): the conformance suite (`crates/core/src/storage/conformance.rs`, run on every engine) gains cases: clear a quota after setting one; save a purge config with one then both retention axes null; create a project with purge enabled and one axis null. Each reads the value back.
- DEC-003 (SPEC-0022): tool level PostgreSQL tests in a new `#[cfg(test)]` module `crates/core/src/tools/pg_null_e2e.rs`, building the tools harness (`crates/core/src/tools/mod.rs:288`, `harness_with_search_and_doc_service` takes a config closure) with `infra.admin` and `infra.meta` on the `postgres` backend (`config.rs:15`, `:351-354`) and the DSN from `MCPFS_TEST_PG_DSN`, a fresh schema per test, skipped with an `eprintln!` when the variable is unset or the `postgres` feature is off (pattern `conformance.rs:1035-1042`). BT-012/013 reuse the stub conversion of `spec_0019_e2e_115_a_word_image_has_an_empty_page` (`crates/core/src/tools/artifacts.rs:1770`).
### Root cause and sites
`rel/postgres.rs:160`. Sites it breaks: `storage/admin.rs:239-240`, `:483-484`, `:494-495`; `storage/meta.rs:712`, `:747`, `:753`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/rel/postgres.rs` | modify binding (+ BT-020 test) |
| `crates/core/src/storage/conformance.rs` | add DEC-002 cases |
| `crates/core/src/tools/pg_null_e2e.rs` | new |
| `crates/core/src/tools/mod.rs` | `#[cfg(test)] mod pg_null_e2e;` |
| `.agent_docs/backends.md` | dialect checklist line, timestamped 2026-10-12 |

## Acceptance tests
> 100% must pass, through `make check` plus the opt in suites above.

### SPEC-0022/BT-001: Clearing a quota on PostgreSQL
- Red before the fix because: `admin.set_project_quota {max_mb: null}` returns `ERR_INTERNAL_ERROR` naming `quota_bytes`.
- Given project `Atlas` and `admin.set_project_quota {project_id: "Atlas", max_mb: 5}` succeeded
- When Ada calls `admin.set_project_quota {project_id: "Atlas", max_mb: null}`
- Then `{"project_id": "Atlas", "max_mb": null}`; and `admin.get_project_quota {project_id: "Atlas"}` reports `max_mb: null`

### SPEC-0022/BT-002: The conformance suite passes on PostgreSQL
- Red before the fix because: `superseded_at` 42804 (`conformance.rs:1045` panics).
- When `postgres_passes_the_conformance_suite` runs with `MCPFS_TEST_PG_DSN` / Then it passes, DEC-002 cases included.

### SPEC-0022/BT-010: Purge config with null retention axes
- When `admin.set_purge_config {project_id: "Atlas", autopurge_enabled: true, use_internal_purge: true, file_retention_days: 30, project_retention_days: null}`, then with both null
- Then both succeed; read back `project_retention_days` null, then both null.

### SPEC-0022/BT-011: Project creation with a null axis
- When `admin.create_project {project_id: "Borea", owner: "olivia@example.com", autopurge_enabled: true, file_retention_days: 7, project_retention_days: null}`
- Then the project exists; config `file_retention_days: 7`, `project_retention_days: null`.

### SPEC-0022/BT-012: A conversion on PostgreSQL keeps its artifacts
- Given the stub conversion of `spec_0019_e2e_115` on `/memo.docx`
- When `fs.documentize` / Then success, and `fs.list_images` returns `{"images": [{"id": "image-1", "caption": "Org chart", "page": null}], "marker": null}`

### SPEC-0022/BT-013: Reconverting supersedes the previous set
- Given BT-012 / When `/memo.docx` is rewritten and converted again / Then `fs.list_images` reports only the new conversion's images.

### SPEC-0022/BT-020: Null into a text column still accepted on PostgreSQL
- `live_migrate_is_idempotent_and_round_trips_every_value_type` (`rel/postgres.rs:428`) stays green; add a null into the `BIGINT` column `n` of the probe table and assert it reads back null.

### SPEC-0022/BT-021: SQLite results unchanged
- `make check` green, previous count plus the new tests.

### SPEC-0022/BT-022: SQL Server conformance unchanged
- `MCPFS_TEST_MSSQL_DSN` set: the SQL Server conformance case passes, DEC-002 cases included. If it fails on a DEC-002 case, that is a separate SQL Server defect: write a backlog entry, do not change `rel/sqlserver.rs`.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/sqlite.rs`, `rel/sqlserver.rs`, the 7 call sites in `storage/admin.rs` and `storage/meta.rs`, `TOOL_CONTRACT.txt`, `tool-contract-golden.json`.
- Dependencies not to add: any (no sqlx feature change).
- Patterns to avoid: SQL casts per site; a new `SqlValue` variant.
- PostgreSQL tests skip, never fail, when `MCPFS_TEST_PG_DSN` is unset.

## Definition of done
- [ ] RED for BT-001, BT-002, BT-010 to BT-013 against the running compose PostgreSQL before the fix.
- [ ] Every acceptance test green; `make check` green.
- [ ] Non regression: `spec_0019_e2e_115_*`, the three conformance cases, `live_migrate_*`.
- [ ] Docs updated: `.agent_docs/backends.md`.
- [ ] Self review: full.
- [ ] Closes spec: every BT- of SPEC-0022 green; BR-001..BR-006 cited.
- [ ] One commit `US-0023: Absent values accepted on PostgreSQL`, body `Spec: SPEC-0022`.
