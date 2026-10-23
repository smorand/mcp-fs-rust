# US-0078: refuse SQL Server in the configuration, with baselines

> Spec: SPEC-0051 (sdlc/specs/SPEC-0051-remove-sql-server-support/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 6
> Closes spec: no

## Objective
A configuration naming `sqlserver` for any store is refused at startup and by `migrate` with one exact message, before anything else is checked or written. This story also captures, on the unmodified code, the baselines every later removal is proved against.

## Requirements covered
- SPEC-0051/FR-NEW-001: IF a store section of the configuration names the database `sqlserver` THEN THE Operations context SHALL refuse to start and print `infra.<section>.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`.
- SPEC-0051/FR-NEW-002: IF either configuration given to the migration command names `sqlserver` for a store THEN THE Operations context SHALL refuse with the message of FR-NEW-001 before reading or writing any data.
- SPEC-0051/FR-MOD-001: WHEN a store section names a database that is neither `sqlite`, `postgres` nor `sqlserver` THE Operations context SHALL refuse to start and print `unknown infra.<section>.backend '<name>', expected one of sqlite, postgres`.
- SPEC-0051/FR-NEW-003 (partial: baselines and previous database): The Storage context SHALL keep, on SQLite and on PostgreSQL, every answer, every limit and the stored layout it had before this change.
- SPEC-0051/FR-DEL-001 (partial: the choice is refused): the choice of SQL Server for any of the four stores; the dedicated refusal of a path longer than a SQL Server column (`path is <N> characters but this backend (sqlserver) allows at most <M>: shorten a directory name or move the entry nearer the volume root, ...`), which can no longer be produced; the migration to or from SQL Server.

## Independent Test
`cargo test -p mcp-fs --test sql_server_removed` spawns the real binary: a configuration whose `infra.meta.backend` is `sqlserver` makes `mcp-fs serve` exit 1 with the last stderr line `error: ERR_INVALID_ARGUMENT: infra.meta.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'` and an empty stdout, nothing written; the golden and the fixture tests pass on the unmodified storage code.

## Design excerpt
### Decisions
- DEC-003 (SPEC-0051): The refusal is a pre-scan called first in `ServerConfig::validate`, not an arm of `validate_store`. **Obliged by:** SPEC-0051/FR-NEW-001. **Rationale:** the spec orders it before every other check (E2E-021, 022) and after the unreadable file errors, which `from_yaml` already raises before `validate`. **Alternatives considered:** an arm in `validate_store` (wrong precedence); a serde level enum (changes the unreadable file text). **Source:** spec. **Evidence:** `crates/core/src/config.rs:940-984`
- DEC-004 (SPEC-0051): `migrate` needs no code: it loads both configurations through the same validation. **Obliged by:** SPEC-0051/FR-NEW-002. **Rationale:** `cmd_migrate` calls `ServerConfig::load(from)` then `(to)` before printing anything. **Alternatives considered:** a second check in the verb (duplicate). **Source:** existing code. **Evidence:** `crates/core/src/cli.rs:168-181`
- DEC-005 (SPEC-0051): The baselines are committed in US-0078 before any removal: (a) golden table definitions of the four stores for SQLite and PostgreSQL, rendered with `SchemaSet::render`; (b) a SQLite state directory written by the unmodified binary, reopened by E2E-001 and E2E-016. **Obliged by:** SPEC-0051/FR-NEW-003. **Rationale:** the spec requires a byte identical layout and a readable previous database; only a capture before the change proves it. **Alternatives considered:** deriving the golden after the change (proves nothing). **Source:** spec 14.3. **Evidence:** `crates/core/src/storage/rel/schema.rs:222`
- DEC-006 (SPEC-0051): The test inventory is two committed text files in the spec directory: `test-inventory-before.txt` (names from `cargo test --workspace -- --list`, default features, sorted, captured in US-0078) and `removed-tests.txt` (the removal list, grown by US-0079 and US-0080). The check, run in the Definition of done of US-0079 to US-0082, is: `after = before − removed` once names beginning `spec_0051_` are ignored. **Obliged by:** SPEC-0051/FR-NEW-003, E2E-002, E2E-020. **Rationale:** a runnable proof that no other test disappears. **Alternatives considered:** counting tests (hides swaps). **Source:** spec 14.3. **Evidence:** n/a
- DEC-014 (SPEC-0051): Acceptance tests that need the process boundary spawn the real binary through `env!("CARGO_BIN_EXE_mcp-fs")` in `crates/mcp-fs/tests/sql_server_removed.rs`; structural checks (lock, compose, docs, cargo feature) live in the same file. **Obliged by:** SPEC-0051/all E2E. **Rationale:** stdout, stderr and exit status are the observables; the pattern of `security_gate.rs` already runs commands from tests. **Alternatives considered:** calling `ServerConfig::from_yaml` only (misses the channel and the exit status). **Source:** existing code. **Evidence:** `crates/mcp-fs/tests/security_gate.rs`
### Data model
No change. The stored layout is byte identical on SQLite and PostgreSQL.
### Contracts
Refusal: stderr last line `error: ERR_INVALID_ARGUMENT: infra.<section>.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`, stdout empty, exit 1; unknown name: `unknown infra.<section>.backend '<name>', expected one of sqlite, postgres`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/config.rs` | modify: `backend::REMOVED = "sqlserver"` replaces `SQLSERVER`; `backend::ALL = [SQLITE, POSTGRES]`; add private `reject_removed_backends(&self)` called first in `validate` (before `max_diff_file_bytes`, `config.rs:967-984`); delete the sqlserver arm of `validate_store` (`1062-1073`); replace its 3 SQL Server tests and update the unknown backend test (`1516`) |
| `crates/mcp-fs/tests/sql_server_removed.rs` | create: spawns `env!("CARGO_BIN_EXE_mcp-fs")` with `serve --config <tmp>` and `migrate --from --to` |
| `crates/core/tests/fixtures/schema_sqlite.sql`, `schema_postgres.sql` | create: golden of `SchemaSet::render` for the four stores (meta, admin, `git::db::schema()`, `git::oauth::persistence::schema()`), one statement per paragraph; compare byte for byte; written by an `#[ignore]` regeneration test run once on unmodified code |
| `crates/core/tests/fixtures/state_v1/` | create: SQLite state written by the unmodified code: project `Atlas`, member, 3 files; reopened by a copy in a temp dir; written by an `#[ignore]` regeneration test |
| `sdlc/specs/SPEC-0051-remove-sql-server-support/test-inventory-before.txt` | create: `cargo test --workspace -- --list` names, sorted, taken before any change |
### Existing patterns
Command running tests: `crates/mcp-fs/tests/security_gate.rs`; config tests: `crates/core/src/config.rs` test module; end to end tools: `crates/core/src/tools/teardown_e2e.rs`. Comments explain why, no dashes as punctuation.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0051/E2E-001: A SQLite server starts and answers
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** Critical
- **Given** the shared setup
- **When** the operator starts the server
- **Then** the health check answers ok and a member of `Atlas` lists its 3 files

### SPEC-0051/E2E-005: The metadata store cannot be SQL Server
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001, FR-DEL-001. **Priority:** Critical
- **Given** a configuration whose `meta` store names `sqlserver` with the connection string `server=h;database=d`
- **When** the operator starts the server
- **Then** it is refused with `ERR_INVALID_ARGUMENT: infra.meta.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`
- **And** no state directory, database file or connection was created

### SPEC-0051/E2E-006: The admin store cannot be SQL Server
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001. **Priority:** Critical
- **Given** a configuration whose `admin` store names `sqlserver` and the other stores `sqlite`
- **When** the operator starts the server
- **Then** it is refused with `ERR_INVALID_ARGUMENT: infra.admin.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`

### SPEC-0051/E2E-007: The oauth store cannot be SQL Server
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001. **Priority:** Critical
- **Given** a configuration whose `oauth` store names `sqlserver` and the other stores `sqlite`
- **When** the operator starts the server
- **Then** it is refused with `ERR_INVALID_ARGUMENT: infra.oauth.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`

### SPEC-0051/E2E-008: Without a connection string the removal message still comes first
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001. **Priority:** High
- **Given** a configuration whose `meta` store names `sqlserver` with no connection string
- **When** the operator starts the server
- **Then** the message is the one of E2E-005, and no message about a missing connection string appears

### SPEC-0051/E2E-009: Other unknown names list two databases
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-MOD-001. **Priority:** High
- **Given** a configuration whose `meta` store names `mssql`, then one naming `oracle`
- **When** the operator starts the server each time
- **Then** the refusals are `ERR_INVALID_ARGUMENT: unknown infra.meta.backend 'mssql', expected one of sqlite, postgres` and `ERR_INVALID_ARGUMENT: unknown infra.meta.backend 'oracle', expected one of sqlite, postgres`

### SPEC-0051/E2E-010: A migration from SQL Server is refused
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-002, FR-DEL-001. **Priority:** High
- **Given** a source configuration whose `meta` store names `sqlserver` and an empty destination PostgreSQL database
- **When** the operator runs the migration command
- **Then** it is refused with the message of E2E-005
- **And** the destination database holds no table written by the migration, the standard output is empty and the standard error does not contain `migrating`

### SPEC-0051/E2E-011: A migration to SQL Server is refused
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-002. **Priority:** High
- **Given** the shared setup as the source and a destination configuration whose `meta` store names `sqlserver`
- **When** the operator runs the migration command
- **Then** it is refused with the message of E2E-005
- **And** the SQLite source is unchanged (still 3 files in `Atlas`), the standard output is empty and the standard error does not contain `migrating`

### SPEC-0051/E2E-013: With several sections naming SQL Server the first is named
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001. **Priority:** High
- **Given** a configuration whose `git` and `oauth` stores name `sqlserver`, and one whose `meta` and `oauth` stores do
- **When** the operator starts the server each time
- **Then** the first refusal names `infra.git.backend` and the second names `infra.meta.backend`

### SPEC-0051/E2E-014: The connection string is never printed
- **Category:** side effect. **Scenario:** SC-002. **Requirements:** FR-NEW-001. **Priority:** Critical
- **Given** a configuration whose `meta` store names `sqlserver` with the connection string `Server=h;Password=Secret1;Database=d`
- **When** the operator starts the server
- **Then** neither the standard output nor the standard error contains `Secret1`, `Server=h` or `Database=d`

### SPEC-0051/E2E-016: The stored layout is unchanged
- **Category:** invariance. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** Critical
- **Given** the table definitions SQLite and PostgreSQL produce for the four stores, captured from the previous version
- **When** the same definitions are produced after this change
- **Then** they are byte identical for SQLite and for PostgreSQL
- **And** a SQLite database written by the previous version opens and lists `Atlas` with its 3 files

### SPEC-0051/E2E-021: The removal refusal comes before any other configuration error
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001, FR-MOD-001. **Priority:** High
- **Given** a configuration whose `meta` store names `postgres` with no connection string and whose `admin` store names `sqlserver`; and one whose `meta` store names `mssql` and whose `admin` store names `sqlserver`
- **When** the operator starts the server each time
- **Then** both refusals are `ERR_INVALID_ARGUMENT: infra.admin.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`

### SPEC-0051/E2E-022: A file that cannot be read keeps its own error, other validation comes after the removal
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001. **Priority:** High
- **Given** a configuration where `safety.max_diff_file_bytes` is 0 and the `admin` store names `sqlserver`; and a configuration file that is not valid YAML and also names `sqlserver`
- **When** the operator starts the server each time
- **Then** the first refusal is `ERR_INVALID_ARGUMENT: infra.admin.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`; the second starts with `error: ERR_INVALID_ARGUMENT: invalid config: ` as today

## Constraints
- Files not to touch: `TOOL_CONTRACT.txt`, `tool-contract-golden.json`, any other spec.
- Dependencies not to add: none.
- Patterns to avoid: hand edited `Cargo.lock`; touching a limit message; scope of later stories.
- Scope boundary: the stories after this one in the queue.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code; characterization tests (already green) get a mutation check instead.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: SQLite and PostgreSQL answers, limits (branch 255 bytes, token 8192) and stored layout unchanged; the golden and the fixture of US-0078 pass untouched.
- [ ] Docs updated: none beyond the story paths.
- [ ] Self review: full.
- [ ] One commit `US-0078: refuse SQL Server in the configuration, with baselines`, body `Spec: SPEC-0051` and the requirement ids.
