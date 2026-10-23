# US-0080: collapse the dialect and the bounded keys

> Spec: SPEC-0051 (sdlc/specs/SPEC-0051-remove-sql-server-support/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0079
> min_tier: 2
> Files touched: 11
> Closes spec: no

## Objective
`Dialect::SqlServer` and every arm on it disappear, bounded text keys become plain text and the SPEC-0050 comparison helpers shrink to the two remaining engines. SQLite and PostgreSQL table definitions stay byte identical.

## Requirements covered
- SPEC-0051/FR-DEL-001 (partial: dialect, schema and bounded keys): the choice of SQL Server for any of the four stores; the dedicated refusal of a path longer than a SQL Server column (`path is <N> characters but this backend (sqlserver) allows at most <M>: shorten a directory name or move the entry nearer the volume root, ...`), which can no longer be produced; the migration to or from SQL Server.
- SPEC-0051/FR-NEW-003 (partial: layout and limits unchanged): The Storage context SHALL keep, on SQLite and on PostgreSQL, every answer, every limit and the stored layout it had before this change.

## Independent Test
`cargo test -p mcp-fs-core` stays green with the golden of US-0078 unchanged (`git diff --stat crates/core/tests/fixtures` empty); `rg -n 'SqlServer|TextKey' crates` prints nothing; the inventory check holds.

## Design excerpt
### Decisions
- DEC-009 (SPEC-0051): `Dialect::SqlServer` and every arm on it are deleted; the SPEC-0050 helpers `char_len` and `exact_equals` and the SQL Server branch of `substr_fn` go, `SubtreeMatch` uses `substr`, `chars().count()` and `=` directly. **Obliged by:** SPEC-0051/FR-DEL-001. **Rationale:** the simplification the change exists for. **Alternatives considered:** keeping arms without a driver (untested dead code). **Source:** existing code. **Evidence:** `crates/core/src/storage/rel/dialect.rs:13-17`, `233-240`, `257-300`
- DEC-010 (SPEC-0051): `ColumnType::TextKey(n)` is folded into `ColumnType::Text` and its `*_LEN` constants removed. **Obliged by:** SPEC-0051/FR-NEW-003 (layout unchanged), FR-DEL-001. **Rationale:** it exists for SQL Server index limits and renders `TEXT` on SQLite and PostgreSQL; the golden proves no definition moves. **Alternatives considered:** keeping the type (a no-op bound). **Source:** existing code. **Evidence:** `crates/core/src/storage/rel/dialect.rs:28`, `218`, `226`
- DEC-012 (SPEC-0051): The tests naming SQL Server are deleted with their subject; shared tests keep their PostgreSQL and SQLite assertions; the conformance `Engine::SqlServer` goes. **Obliged by:** SPEC-0051/FR-DEL-001, E2E-020. **Rationale:** DEC-006 proves nothing else disappears. **Alternatives considered:** `#[ignore]` (keeps dead code). **Source:** spec. **Evidence:** `crates/core/src/storage/conformance.rs:43-95`
- DEC-011 (SPEC-0051): The SQL Server path limit goes: `max_path_len`, `ensure_path_fits`, `ensure_storable_path` and the `SafetyManager::new` parameter. The limits that apply everywhere stay with their messages: branch name 255 bytes and token 8 192 characters, only their comments are corrected. **Obliged by:** SPEC-0051/FR-DEL-001, FR-NEW-003. **Rationale:** `max_path_len` is `None` on every remaining backend. **Alternatives considered:** keeping the parameter as `Option` always `None` (dead surface). **Source:** existing code. **Evidence:** `crates/core/src/safety.rs:101-115`, `crates/core/src/tools/git.rs:2195`, `crates/core/src/tools/git_auth.rs:27`
- DEC-006 (SPEC-0051): The test inventory is two committed text files in the spec directory: `test-inventory-before.txt` (names from `cargo test --workspace -- --list`, default features, sorted, captured in US-0078) and `removed-tests.txt` (the removal list, grown by US-0079 and US-0080). The check, run in the Definition of done of US-0079 to US-0082, is: `after = before − removed` once names beginning `spec_0051_` are ignored. **Obliged by:** SPEC-0051/FR-NEW-003, E2E-002, E2E-020. **Rationale:** a runnable proof that no other test disappears. **Alternatives considered:** counting tests (hides swaps). **Source:** spec 14.3. **Evidence:** n/a
### Data model
No change. The stored layout is byte identical on SQLite and PostgreSQL.
### Contracts
Refusal: stderr last line `error: ERR_INVALID_ARGUMENT: infra.<section>.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`, stdout empty, exit 1; unknown name: `unknown infra.<section>.backend '<name>', expected one of sqlite, postgres`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/rel/dialect.rs`, `schema.rs`, `postgres.rs`, `sqlite.rs` | modify: delete the variant, its arms, `char_len`, `exact_equals`, the SQL Server branch of `substr_fn`, `ColumnType::TextKey` and its tests |
| `crates/core/src/storage/meta.rs`, `admin.rs`, `git/db.rs`, `git/oauth/persistence.rs` | modify: `TextKey(n)` becomes `Text`, drop the `*_LEN` constants; `SubtreeMatch` uses `substr`, `chars().count()` and `=` directly |
| `crates/core/src/tools/git.rs`, `tools/git_auth.rs`, `migrate.rs` | modify: comments only (limits and batch size keep their values and messages) |
| `sdlc/specs/SPEC-0051-remove-sql-server-support/removed-tests.txt` | modify: append the removed names |
### Existing patterns
Command running tests: `crates/mcp-fs/tests/security_gate.rs`; config tests: `crates/core/src/config.rs` test module; end to end tools: `crates/core/src/tools/teardown_e2e.rs`. Comments explain why, no dashes as punctuation.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0051/E2E-017: The branch name limit is unchanged
- **Category:** invariance. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** High
- **Given** a project with git enabled
- **When** a member creates a branch named with 255 bytes, then one with 256 bytes
- **Then** the first is accepted and the second is refused with `'<name>' is not a valid branch name: it is 256 bytes of UTF-8, over the 255 byte limit` (with the 256 byte name in place of `<name>`), the same message as before this change

### SPEC-0051/E2E-018: The token limit is unchanged
- **Category:** invariance. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** High
- **Given** git enabled and a member seeding a token
- **When** the member seeds a token of 8 192 characters, then one of 8 193
- **Then** the first is accepted and the second is refused with `token must be at most 8192 characters`, the same message as before this change

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
- [ ] One commit `US-0080: collapse the dialect and the bounded keys`, body `Spec: SPEC-0051` and the requirement ids.
