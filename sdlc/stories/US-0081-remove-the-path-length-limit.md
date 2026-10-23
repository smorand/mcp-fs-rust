# US-0081: remove the path length limit

> Spec: SPEC-0051 (sdlc/specs/SPEC-0051-remove-sql-server-support/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0080
> min_tier: 2
> Files touched: 17
> Closes spec: no

## Objective
The refusal of a path longer than a SQL Server column can no longer be produced, so its machinery goes: `max_path_len`, the path check and the second parameter of `SafetyManager::new`. A 2 000 character path stays accepted. Mechanical change over 37 call sites that the compiler checks; the unit cannot be cut thinner without breaking the build, so it is one story.

## Requirements covered
- SPEC-0051/FR-DEL-001 (partial: the dedicated path refusal): the choice of SQL Server for any of the four stores; the dedicated refusal of a path longer than a SQL Server column (`path is <N> characters but this backend (sqlserver) allows at most <M>: shorten a directory name or move the entry nearer the volume root, ...`), which can no longer be produced; the migration to or from SQL Server.
- SPEC-0051/FR-NEW-003 (partial: no path limit on SQLite and PostgreSQL): The Storage context SHALL keep, on SQLite and on PostgreSQL, every answer, every limit and the stored layout it had before this change.

## Independent Test
`rg -n 'max_path_len|ensure_path_fits|ensure_storable_path' crates` prints nothing; a 2 000 character path writes and reads back on SQLite; `make check` is green; the inventory check holds.

## Design excerpt
### Decisions
- DEC-011 (SPEC-0051): The SQL Server path limit goes: `max_path_len`, `ensure_path_fits`, `ensure_storable_path` and the `SafetyManager::new` parameter. The limits that apply everywhere stay with their messages: branch name 255 bytes and token 8 192 characters, only their comments are corrected. **Obliged by:** SPEC-0051/FR-DEL-001, FR-NEW-003. **Rationale:** `max_path_len` is `None` on every remaining backend. **Alternatives considered:** keeping the parameter as `Option` always `None` (dead surface). **Source:** existing code. **Evidence:** `crates/core/src/safety.rs:101-115`, `crates/core/src/tools/git.rs:2195`, `crates/core/src/tools/git_auth.rs:27`
- DEC-012 (SPEC-0051): The tests naming SQL Server are deleted with their subject; shared tests keep their PostgreSQL and SQLite assertions; the conformance `Engine::SqlServer` goes. **Obliged by:** SPEC-0051/FR-DEL-001, E2E-020. **Rationale:** DEC-006 proves nothing else disappears. **Alternatives considered:** `#[ignore]` (keeps dead code). **Source:** spec. **Evidence:** `crates/core/src/storage/conformance.rs:43-95`
- DEC-006 (SPEC-0051): The test inventory is two committed text files in the spec directory: `test-inventory-before.txt` (names from `cargo test --workspace -- --list`, default features, sorted, captured in US-0078) and `removed-tests.txt` (the removal list, grown by US-0079 and US-0080). The check, run in the Definition of done of US-0079 to US-0082, is: `after = before − removed` once names beginning `spec_0051_` are ignored. **Obliged by:** SPEC-0051/FR-NEW-003, E2E-002, E2E-020. **Rationale:** a runnable proof that no other test disappears. **Alternatives considered:** counting tests (hides swaps). **Source:** spec 14.3. **Evidence:** n/a
### Data model
No change. The stored layout is byte identical on SQLite and PostgreSQL.
### Contracts
Refusal: stderr last line `error: ERR_INVALID_ARGUMENT: infra.<section>.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`, stdout empty, exit 1; unknown name: `unknown infra.<section>.backend '<name>', expected one of sqlite, postgres`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/safety.rs` | modify: delete `max_path_len`, `ensure_path_fits` and their 5 tests; `SafetyManager::new(config)` loses its second argument |
| `crates/core/src/storage/meta.rs` | modify: delete `max_path_len`, `ensure_storable_path`, `PATH_LEN`, `MAX_PATH_CHARS` and their tests |
| the 37 call sites of `SafetyManager::new(` (`cli.rs`, `app.rs`, `api/dataplane.rs`, `api/openapi.rs`, `git/http/mod.rs`, `tools/*`) | modify: drop the second argument (`rg -n 'SafetyManager::new\(' crates`) |
| `sdlc/specs/SPEC-0051-remove-sql-server-support/removed-tests.txt` | modify: append the removed names |
### Existing patterns
Command running tests: `crates/mcp-fs/tests/security_gate.rs`; config tests: `crates/core/src/config.rs` test module; end to end tools: `crates/core/src/tools/teardown_e2e.rs`. Comments explain why, no dashes as punctuation.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0051/E2E-019: A very long path is accepted on SQLite
- **Category:** invariance. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** High
- **Given** the shared setup
- **When** a member writes a file under a path of 2 000 characters
- **Then** the write is accepted and the file reads back with its content

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
- [ ] One commit `US-0081: remove the path length limit`, body `Spec: SPEC-0051` and the requirement ids.
