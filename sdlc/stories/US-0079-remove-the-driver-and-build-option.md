# US-0079: remove the driver, the build option and the test service

> Spec: SPEC-0051 (sdlc/specs/SPEC-0051-remove-sql-server-support/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0078
> min_tier: 2
> Files touched: 14
> Closes spec: no

## Objective
The SQL Server driver, its pool, the build option, the optional dependencies, the lock entries and the test container are removed. PostgreSQL and SQLite builds and suites are unchanged.

## Requirements covered
- SPEC-0051/FR-DEL-002: the build option `sqlserver`, the SQL Server container of the test environment and the SQL Server opt in suite; the build option `all-backends` stays and now means PostgreSQL and vector search.
- SPEC-0051/FR-DEL-001 (partial: the driver, the pool and the SQL Server only tests): the choice of SQL Server for any of the four stores; the dedicated refusal of a path longer than a SQL Server column (`path is <N> characters but this backend (sqlserver) allows at most <M>: shorten a directory name or move the entry nearer the volume root, ...`), which can no longer be produced; the migration to or from SQL Server.
- SPEC-0051/FR-NEW-003 (partial: PostgreSQL suite unchanged): The Storage context SHALL keep, on SQLite and on PostgreSQL, every answer, every limit and the stored layout it had before this change.

## Independent Test
`cargo check -p mcp-fs --features sqlserver` fails with cargo's unknown feature error; `rg -c 'tiberius|bb8' Cargo.lock` prints 0; `docker compose -f docker-compose.test.yml config --services` lists no `mssql`; `make check` is green and the inventory check (DEC-006) holds.

## Design excerpt
### Decisions
- DEC-007 (SPEC-0051): Build: delete the `sqlserver` feature from both crates, set `all-backends = ["postgres", "rag"]`, delete the three optional dependencies and their workspace entries after `cargo tree -i tokio-util` confirms no other user, regenerate `Cargo.lock` with cargo. **Obliged by:** SPEC-0051/FR-DEL-002. **Rationale:** the spec keeps `all-backends`. **Alternatives considered:** dropping `all-backends` (breaks nothing in the repo but the spec keeps it). **Source:** spec. **Evidence:** `crates/core/Cargo.toml:19-27`, `crates/mcp-fs/Cargo.toml:15-20`, `Cargo.toml:43-56`
- DEC-008 (SPEC-0051): The storage factories keep only `sqlite` and `postgres`; the `cfg(any(postgres, sqlserver))` gates become `cfg(postgres)`. **Obliged by:** SPEC-0051/FR-DEL-001. **Rationale:** no other user of the SQL Server gates. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `crates/core/src/storage/mod.rs:45-76`, `149-185`
- DEC-012 (SPEC-0051): The tests naming SQL Server are deleted with their subject; shared tests keep their PostgreSQL and SQLite assertions; the conformance `Engine::SqlServer` goes. **Obliged by:** SPEC-0051/FR-DEL-001, E2E-020. **Rationale:** DEC-006 proves nothing else disappears. **Alternatives considered:** `#[ignore]` (keeps dead code). **Source:** spec. **Evidence:** `crates/core/src/storage/conformance.rs:43-95`
- DEC-006 (SPEC-0051): The test inventory is two committed text files in the spec directory: `test-inventory-before.txt` (names from `cargo test --workspace -- --list`, default features, sorted, captured in US-0078) and `removed-tests.txt` (the removal list, grown by US-0079 and US-0080). The check, run in the Definition of done of US-0079 to US-0082, is: `after = before − removed` once names beginning `spec_0051_` are ignored. **Obliged by:** SPEC-0051/FR-NEW-003, E2E-002, E2E-020. **Rationale:** a runnable proof that no other test disappears. **Alternatives considered:** counting tests (hides swaps). **Source:** spec 14.3. **Evidence:** n/a
- DEC-014 (SPEC-0051): Acceptance tests that need the process boundary spawn the real binary through `env!("CARGO_BIN_EXE_mcp-fs")` in `crates/mcp-fs/tests/sql_server_removed.rs`; structural checks (lock, compose, docs, cargo feature) live in the same file. **Obliged by:** SPEC-0051/all E2E. **Rationale:** stdout, stderr and exit status are the observables; the pattern of `security_gate.rs` already runs commands from tests. **Alternatives considered:** calling `ServerConfig::from_yaml` only (misses the channel and the exit status). **Source:** existing code. **Evidence:** `crates/mcp-fs/tests/security_gate.rs`
### Data model
No change. The stored layout is byte identical on SQLite and PostgreSQL.
### Contracts
Refusal: stderr last line `error: ERR_INVALID_ARGUMENT: infra.<section>.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`, stdout empty, exit 1; unknown name: `unknown infra.<section>.backend '<name>', expected one of sqlite, postgres`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/rel/sqlserver.rs`, `crates/core/src/tools/mssql_trash_e2e.rs` | delete |
| `crates/core/src/storage/mod.rs`, `rel/mod.rs`, `conformance.rs`, `tools/mod.rs`, `tools/teardown_e2e.rs`, `lib.rs`, `storage/meta.rs` | modify: remove `build_sqlserver`, the `cfg(feature = "sqlserver")` items and tests, `Engine::SqlServer`; `cfg(any(postgres, sqlserver))` becomes `cfg(postgres)` (`storage/mod.rs:45-76`, `149-185`) |
| `Cargo.toml`, `crates/core/Cargo.toml`, `crates/mcp-fs/Cargo.toml`, `Cargo.lock` | modify: drop feature `sqlserver`, optional `tiberius-ng`, `bb8`, `tokio-util` (after `cargo tree -i tokio-util`); `all-backends = ["postgres", "rag"]`; regenerate the lock with cargo |
| `docker-compose.test.yml` | modify: delete the `mssql` service and its comments (`39-54`) |
| `crates/mcp-fs/tests/sql_server_removed.rs` | modify: add the cargo feature, lock and compose tests |
| `sdlc/specs/SPEC-0051-remove-sql-server-support/removed-tests.txt` | create: names of the tests removed by this story |
### Existing patterns
Command running tests: `crates/mcp-fs/tests/security_gate.rs`; config tests: `crates/core/src/config.rs` test module; end to end tools: `crates/core/src/tools/teardown_e2e.rs`. Comments explain why, no dashes as punctuation.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0051/E2E-002: The PostgreSQL suite passes unchanged
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** Critical
- **Given** the PostgreSQL service of the test environment is up
- **When** the developer runs the PostgreSQL end to end suite
- **Then** every test passes, and the set of test names of the suite equals the inventory recorded before this change (Section 14.3, b) minus exactly the tests of the removal list

### SPEC-0051/E2E-003: The migration from SQLite to PostgreSQL still copies the state
- **Category:** happy. **Scenario:** SC-003. **Requirements:** FR-NEW-003. **Priority:** High
- **Given** the shared setup and an empty PostgreSQL database
- **When** the operator migrates the SQLite configuration to the PostgreSQL configuration
- **Then** `Atlas` exists on PostgreSQL with the same 3 files and the same sizes

### SPEC-0051/E2E-012: The removed build option is refused
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-DEL-002. **Priority:** High
- **Given** a checkout of the project
- **When** the builder runs `cargo build --features sqlserver`
- **Then** cargo exits with a non zero status and says the feature `sqlserver` is not found
- **And** `cargo build --features all-backends` still resolves its features

### SPEC-0051/E2E-015: The test environment no longer offers SQL Server
- **Category:** side effect. **Scenario:** SC-004. **Requirements:** FR-DEL-002. **Priority:** Medium
- **Given** the test environment definition
- **When** the developer lists its services
- **Then** the list is `postgres` only and no service is named `mssql`
- **And** the dependency lock holds no package named `tiberius-ng`, `tiberius-ng-macros`, `tiberius` or `bb8` (or any package whose name starts with `tiberius`)

## Constraints
- Files not to touch: `TOOL_CONTRACT.txt`, `tool-contract-golden.json`, any other spec.
- Dependencies not to add: none.
- Patterns to avoid: hand edited `Cargo.lock`; touching a limit message; scope of later stories.
- Scope boundary: the stories after this one in the queue.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code; characterization tests (already green) get a mutation check instead.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: SQLite and PostgreSQL answers, limits (branch 255 bytes, token 8192) and stored layout unchanged; the golden and the fixture of US-0078 pass untouched.
- [ ] Inventory check (DEC-006): after minus `spec_0051_` tests equals before minus `removed-tests.txt`.
- [ ] Docs updated: none beyond the story paths.
- [ ] Self review: full.
- [ ] One commit `US-0079: remove the driver, the build option and the test service`, body `Spec: SPEC-0051` and the requirement ids.
