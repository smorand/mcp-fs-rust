# US-0082: documentation and spec close

> Spec: SPEC-0051 (sdlc/specs/SPEC-0051-remove-sql-server-support/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0081
> min_tier: 2
> Files touched: 10
> Closes spec: yes

## Objective
The README, the agent index, the reference documents and the test environment comments present two databases and no SQL Server instruction. This story closes the spec: the full acceptance set and the requirement coverage audit.

## Requirements covered
- SPEC-0051/FR-NEW-004: The Delivery context SHALL document SQLite and PostgreSQL as the supported databases, and SHALL describe SQL Server only as removed.
- SPEC-0051/FR-NEW-003 (partial: the existing suite passes): The Storage context SHALL keep, on SQLite and on PostgreSQL, every answer, every limit and the stored layout it had before this change.

## Independent Test
`cargo test -p mcp-fs --test sql_server_removed` (documentation scan); `make check` green; the inventory check ends with after = before minus removed; every `E2E-` of the spec is green.

## Design excerpt
### Decisions
- DEC-013 (SPEC-0051): Documentation: every file under `README.md`, `AGENTS.md`, `.agent_docs/` and the compose comments drops SQL Server; the README gets exactly the sentence `SQL Server support was removed on 2026-10-23 (SPEC-0051).`; `lineage.md` may keep dated lines. **Obliged by:** SPEC-0051/FR-NEW-004. **Rationale:** the spec fixes the sentence. **Alternatives considered:** a longer migration note. **Source:** spec. **Evidence:** `README.md:9`, `.agent_docs/backends.md:149-218`
- DEC-006 (SPEC-0051): The test inventory is two committed text files in the spec directory: `test-inventory-before.txt` (names from `cargo test --workspace -- --list`, default features, sorted, captured in US-0078) and `removed-tests.txt` (the removal list, grown by US-0079 and US-0080). The check, run in the Definition of done of US-0079 to US-0082, is: `after = before − removed` once names beginning `spec_0051_` are ignored. **Obliged by:** SPEC-0051/FR-NEW-003, E2E-002, E2E-020. **Rationale:** a runnable proof that no other test disappears. **Alternatives considered:** counting tests (hides swaps). **Source:** spec 14.3. **Evidence:** n/a
- DEC-014 (SPEC-0051): Acceptance tests that need the process boundary spawn the real binary through `env!("CARGO_BIN_EXE_mcp-fs")` in `crates/mcp-fs/tests/sql_server_removed.rs`; structural checks (lock, compose, docs, cargo feature) live in the same file. **Obliged by:** SPEC-0051/all E2E. **Rationale:** stdout, stderr and exit status are the observables; the pattern of `security_gate.rs` already runs commands from tests. **Alternatives considered:** calling `ServerConfig::from_yaml` only (misses the channel and the exit status). **Source:** existing code. **Evidence:** `crates/mcp-fs/tests/security_gate.rs`
### Data model
No change. The stored layout is byte identical on SQLite and PostgreSQL.
### Contracts
Refusal: stderr last line `error: ERR_INVALID_ARGUMENT: infra.<section>.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`, stdout empty, exit 1; unknown name: `unknown infra.<section>.backend '<name>', expected one of sqlite, postgres`.
### Paths
| Path | Action |
|---|---|
| `README.md` | modify: databases `sqlite | postgres`; the one sentence `SQL Server support was removed on 2026-10-23 (SPEC-0051).` |
| `AGENTS.md` | modify: overview, features, structure, conventions, tool contract lines |
| `.agent_docs/backends.md`, `testing.md`, `architecture.md`, `config.md`, `search.md`, `git.md`, `lineage.md` | modify: remove SQL Server instructions; `lineage.md` may keep dated lines |
| `crates/mcp-fs/tests/sql_server_removed.rs` | modify: add the documentation scan (`sqlserver`, `SQL Server`, `mssql`, `tiberius`, `bb8`, excluding `sdlc/`, `plan/`, `Cargo.lock`) for E2E-004 |
### Existing patterns
Command running tests: `crates/mcp-fs/tests/security_gate.rs`; config tests: `crates/core/src/config.rs` test module; end to end tools: `crates/core/src/tools/teardown_e2e.rs`. Comments explain why, no dashes as punctuation.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0051/E2E-004: The documentation lists two databases
- **Category:** happy. **Scenario:** SC-005. **Requirements:** FR-NEW-004. **Priority:** Medium
- **Given** the README, the agent index (`AGENTS.md`), every reference document under the agent documents folder and the comments of the test environment definition
- **When** the maintainer searches them for `sqlserver`, `SQL Server`, `mssql`, `tiberius` and `bb8`
- **Then** every hit of `sqlserver`, `SQL Server`, `mssql`, `tiberius` or `bb8` is the removal sentence `SQL Server support was removed on 2026-10-23 (SPEC-0051).`, the refusal message of FR-NEW-001, or a dated line of the history document; none is an instruction, an example, a driver mention or a test command (the scan excludes `sdlc/`, `plan/` and `Cargo.lock`)

### SPEC-0051/E2E-020: The existing suite passes
- **Category:** regression. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** Critical
- **Given** the repository after this change
- **When** the developer runs the full quality gate
- **Then** it passes; the set of test names equals the inventory recorded before this change minus exactly the removal list, so no other test is gone; and the search tool tests pass on SQLite

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
- [ ] Docs updated: README.md, AGENTS.md, .agent_docs/*.
- [ ] Self review: full.
- [ ] Closes spec: the full acceptance set green (E2E-001 to E2E-022) and every requirement of SPEC-0051 cited by a story done or landed.
- [ ] One commit `US-0082: documentation and spec close`, body `Spec: SPEC-0051` and the requirement ids.
