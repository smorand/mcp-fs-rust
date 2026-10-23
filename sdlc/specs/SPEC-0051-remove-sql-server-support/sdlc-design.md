# SPEC-0051: Remove SQL Server support, design

> Spec: sdlc/specs/SPEC-0051-remove-sql-server-support/sdlc-spec.md
> Nature: FEAT
> Depth: L
> Security: n/a
> Designed on: 2026-10-23
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Language, persistence | existing Rust workspace; `RelationalDb`/`Dialect` seam over SQLite and PostgreSQL | per `Cargo.toml` | DEC-001 |

No dependency is added; three are removed (`tiberius-ng`, `bb8`, `tokio-util`), so there is no context7
lookup. Skill loaded: `rust` (`make check` is the gate, `ToolError` in the library, functional e2e on the real
entry points, `[workspace.dependencies]` single source of versions). Project state: existing
(`git ls-files crates | wc -l` far above scaffolding).

## 2. Approaches

### Chosen: A, a staged removal: refuse first, then delete the driver, then simplify, then document

1. **Refusal and baselines** (US-0078): the baselines are committed first on unmodified code (schema golden,
   SQLite state fixture), then a pre-scan in the configuration check refuses `sqlserver` with the exact message.
   After it, nothing can reach the SQL Server code: it is dead but still compiled.
2. **Driver, build and test environment** (US-0079): the driver file, the factory arms, the build option, the
   dependencies, the lock, the compose service and the SQL Server only tests go.
3. **Dialect and schema** (US-0080): `Dialect::SqlServer` and every `match` arm on it go; bounded keys
   (`TextKey`) become plain text, with the golden proving the SQLite and PostgreSQL table definitions did not move.
4. **Path limit** (US-0081): `max_path_len`, the path refusal and the `Safety::new` parameter go (mechanical, many
   call sites).
5. **Documentation and close** (US-0082).

- Pros: each landing leaves `main` green; the user visible refusal ships first; the large deletions come with a
  proof (golden, fixture, inventory) captured before; each commit stays reviewable.
- Cons and risks: five stories; between US-0078 and US-0079 dead SQL Server code remains compiled.
- Affected scope: `config.rs`, `cli.rs` (none), `storage/` (`mod.rs`, `rel/*`, `meta.rs`, `admin.rs`, `conformance.rs`),
  `git/db.rs`, `git/oauth/persistence.rs`, `safety.rs` and its 37 call sites, tests, three manifests, compose, docs.

### Rejected: B, one commit

Rejected (DEC-002): about 60 files in one squash cannot be reviewed, and a failure cannot be bisected.

### Rejected: C, delete the code first, refuse last

Rejected (DEC-002): a window where a SQL Server configuration reaches a missing driver and fails with an
unrelated message; the spec fixes the refusal text.

### Rejected: D, keep the dialect arms, delete only the driver

Rejected (DEC-009): the point of the change is simpler code and tests; arms with no driver would be untested
dead code.

## 3. Architecture

```
 mcp-fs serve / mcp-fs migrate
        |
        v
 ServerConfig::load -> from_yaml (read, ${VAR} expansion)      unreadable file: today's error, unchanged
        |
        v
 ServerConfig::validate
     1. reject_removed_backends()     NEW: scan meta, admin, git, oauth in that order;
                                      first section naming 'sqlserver' -> ToolError::invalid_argument(<message>)
     2. max_diff_file_bytes check     (existing)
     3. validate_store x4             (existing; 'sqlserver' arm deleted, unknown names list "sqlite, postgres")
        |
        v
 storage factories build_meta_store / build_admin_store ...   only 'sqlite' and 'postgres' remain
```

Existing structure: validation order at `crates/core/src/config.rs:967-984`, `validate_store` at `config.rs:1037-1080`,
the load path at `config.rs:940-957`, the CLI printing `error: {e}` at `crates/core/src/cli.rs:131-135`, the
migrate verb at `cli.rs:168-181`. Bounded contexts: Storage (backends and layout), Operations (config, startup,
migrate), Delivery (build, test environment, docs).

## 4. Data model

No change. The stored layout of SQLite and PostgreSQL stays byte identical (E2E-016). `ColumnType::TextKey(n)`
renders as `TEXT` on both engines today (`crates/core/src/storage/rel/dialect.rs:218`, `226`), so folding it
into `ColumnType::Text` changes no definition. No migration, no rollback needed.

## 5. Contracts and interfaces

### 5.1 The refusal

`ServerConfig::reject_removed_backends(&self) -> Result<()>` (private, called first in `validate`):

```rust
const REMOVED: &str = "sqlserver";            // in `backend`, replaces SQLSERVER
for (section, backend) in [("meta", &self.infra.meta.backend), ("admin", &self.infra.admin.backend),
                           ("git", &self.infra.git.backend), ("oauth", &self.infra.oauth.backend)] {
    if backend == backend::REMOVED {
        return Err(ToolError::invalid_argument(format!(
            "infra.{section}.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'")));
    }
}
```

Printed by the existing path as `error: ERR_INVALID_ARGUMENT: <message>` on stderr, exit status 1
(`cli.rs:131-135`, `errors.rs:159-162`). The dsn is never read by it.

### 5.2 The unknown name message

`backend::ALL` becomes `[SQLITE, POSTGRES]`; the unchanged format at `config.rs:1076-1079` then prints
`unknown infra.<section>.backend '<name>', expected one of sqlite, postgres`.

### 5.3 Build options

`sqlserver` is removed from `crates/core/Cargo.toml` and `crates/mcp-fs/Cargo.toml`;
`all-backends = ["postgres", "rag"]` in both; the optional dependencies `tiberius-ng`, `bb8`, `tokio-util` and the
workspace entries (`Cargo.toml:43-56`) go. `Cargo.lock` is regenerated by cargo, never hand edited.

### 5.4 Removed internal surface (checked by the compiler)

`Dialect::SqlServer`, `Dialect::{substr_fn, char_len, exact_equals}` (their two remaining dialects need no
helper: `SubtreeMatch` uses `substr(...)`, `chars().count()` and `=` directly), `ColumnType::TextKey`,
`storage::rel::sqlserver`, `build_sqlserver`, `meta::{max_path_len, ensure_storable_path, PATH_LEN, MAX_PATH_CHARS}`,
`Safety::{max_path_len, ensure_path_fits}` and the second argument of `SafetyManager::new`.

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/config.rs` | modify: `reject_removed_backends`, `REMOVED`, `ALL`, sqlserver arm and its 3 tests removed, unknown name test updated | FR-NEW-001, FR-MOD-001 | US-0078 |
| `crates/mcp-fs/tests/sql_server_removed.rs` | create: CLI tests (spawn the binary), lock, compose, cargo feature, docs checks | FR-NEW-001, 002, FR-DEL-002, FR-NEW-004 | US-0078 to US-0082 |
| `crates/core/tests/fixtures/state_v1/` | create: a SQLite state written by the unmodified binary (project `Atlas`, 3 files) | FR-NEW-003 | US-0078 |
| `crates/core/tests/fixtures/schema_sqlite.sql`, `schema_postgres.sql` | create: the golden table definitions | FR-NEW-003 | US-0078 |
| `crates/core/src/storage/rel/schema.rs` | modify (golden test; later SQL Server arms and tests removed) | FR-NEW-003 | US-0078, US-0080 |
| `crates/core/src/storage/rel/sqlserver.rs`, `crates/core/src/tools/mssql_trash_e2e.rs` | delete | FR-DEL-001, 002 | US-0079 |
| `crates/core/src/storage/mod.rs`, `rel/mod.rs`, `conformance.rs`, `tools/mod.rs`, `tools/teardown_e2e.rs`, `lib.rs`, `meta.rs` (test cfgs) | modify | FR-DEL-001, 002 | US-0079 |
| `Cargo.toml`, `crates/core/Cargo.toml`, `crates/mcp-fs/Cargo.toml`, `Cargo.lock` | modify | FR-DEL-002 | US-0079 |
| `docker-compose.test.yml` | modify (service and comments) | FR-DEL-002 | US-0079 |
| `crates/core/src/storage/rel/dialect.rs`, `schema.rs`, `postgres.rs`, `sqlite.rs`, `meta.rs`, `admin.rs`, `git/db.rs`, `git/oauth/persistence.rs`, `tools/git.rs`, `tools/git_auth.rs`, `migrate.rs` | modify (variant, arms, `TextKey`, comments) | FR-DEL-001, FR-NEW-003 | US-0080 |
| `crates/core/src/safety.rs` and its 37 `SafetyManager::new(` call sites in `cli.rs`, `app.rs`, `api/*`, `git/http/mod.rs`, `tools/*`, tests | modify (mechanical) | FR-DEL-001 | US-0081 |
| `README.md`, `AGENTS.md`, `.agent_docs/*.md` | modify | FR-NEW-004 | US-0082 |

Affected existing tests: removed with the database: `mssql_trash_e2e.rs` (1), the SQL Server cases of
`teardown_e2e.rs`, `conformance.rs`, `dialect.rs` (16 functions mention it), `schema.rs`, `meta.rs`, `safety.rs`,
`config.rs` (3), `sqlserver.rs` (10); modified: the unknown backend test (`config.rs:1516`) and the SPEC-0050
clause tests. Declared in the commit bodies; the inventory check (DEC-006) proves nothing else disappears.

## 7. Compatibility and risks

- Consumers: an operator whose configuration names `sqlserver` now gets the refusal (declared, FR-NEW-001); nobody
  is deployed on it. SQLite and PostgreSQL behavior unchanged, proven by the golden, the fixture and the invariance
  tests.
- Risk: deleting `meta::max_path_len` removes `Safety`'s second parameter at 37 call sites (US-0081). Mitigation:
  a mechanical change the compiler checks, one story.
- Risk: removing the `sqlserver` feature breaks a script using `--features sqlserver`. None exists in the repo
  (`rg sqlserver Makefile scripts build.sh run.sh test.sh` prints nothing).
- Ordering: US-0078 must land first (it creates the baselines on unmodified code); US-0079 before US-0080 (the
  dialect arms are still used by the driver until it goes).
- Rollback: revert the commits; SQL Server never had deployed data.

## 9. Test strategy

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-005, 006, 007, 008, 009, 013, 014, 021, 022 | functional e2e | the real `mcp-fs serve` binary (spawn, read stdout, stderr, exit status) | `crates/mcp-fs/tests/sql_server_removed.rs` | US-0078 |
| E2E-010, 011 | functional e2e | the real `mcp-fs migrate` binary | same | US-0078 |
| E2E-001, 016 | functional e2e / golden | server on the committed fixture; DDL golden compare | `crates/core/tests/` and the same file | US-0078 |
| E2E-002, 003, 012, 015 | functional e2e / structural | PostgreSQL suite and `migrate` (opt in), `cargo check --features sqlserver`, lock and compose text checks | same file, `crates/core/tests/full_stack_e2e.rs` | US-0079 |
| E2E-017, 018 | functional e2e | `McpServer` git tools | `crates/core/src/tools/git*.rs` test modules (existing pattern) | US-0080 |
| E2E-019 | functional e2e | `McpServer::fs_write` with a 2 000 character path | `crates/core/src/tools/` e2e file | US-0081 |
| E2E-004, 020 | structural / regression | text scan of the docs; `make check` with the inventory check | same file | US-0082 |

Test data: the SQLite fixture and the golden files are generated once, by ignored regeneration tests run on the
unmodified code in US-0078. Tests already green when their story starts (invariance, characterization) get a
mutation check instead of RED evidence. E2E-002/003 need `MCPFS_TEST_PG_DSN` (opt in); the default run asserts the
SQLite half.

## 10. Decisions log

- **DEC-001:** No stack change. **Obliged by:** n/a (no code impact): a removal. **Rationale:** conventions fix
  the stack. **Alternatives considered:** none. **Source:** `Cargo.toml`, `AGENTS.md`. **Evidence:** n/a
- **DEC-002:** Staged removal in five stories, refusal first. **Obliged by:** SPEC-0051/FR-NEW-001, FR-DEL-001,
  FR-DEL-002. **Rationale:** each landing stays green, the user visible refusal exists before any code goes,
  baselines are captured before any layout related change. **Alternatives considered:** one commit; refusal last.
  **Source:** design. **Evidence:** n/a
- **DEC-003:** The refusal is a pre-scan called first in `ServerConfig::validate`, not an arm of `validate_store`.
  **Obliged by:** SPEC-0051/FR-NEW-001. **Rationale:** the spec orders it before every other check (E2E-021, 022)
  and after the unreadable file errors, which `from_yaml` already raises before `validate`. **Alternatives
  considered:** an arm in `validate_store` (wrong precedence); a serde level enum (changes the unreadable file
  text). **Source:** spec. **Evidence:** `crates/core/src/config.rs:940-984`
- **DEC-004:** `migrate` needs no code: it loads both configurations through the same validation. **Obliged by:**
  SPEC-0051/FR-NEW-002. **Rationale:** `cmd_migrate` calls `ServerConfig::load(from)` then `(to)` before printing
  anything. **Alternatives considered:** a second check in the verb (duplicate). **Source:** existing code.
  **Evidence:** `crates/core/src/cli.rs:168-181`
- **DEC-005:** The baselines are committed in US-0078 before any removal: (a) golden table definitions of the four
  stores for SQLite and PostgreSQL, rendered with `SchemaSet::render`; (b) a SQLite state directory written by the
  unmodified binary, reopened by E2E-001 and E2E-016. **Obliged by:** SPEC-0051/FR-NEW-003. **Rationale:** the spec
  requires a byte identical layout and a readable previous database; only a capture before the change proves it.
  **Alternatives considered:** deriving the golden after the change (proves nothing). **Source:** spec 14.3.
  **Evidence:** `crates/core/src/storage/rel/schema.rs:222`
- **DEC-006:** The test inventory is two committed text files in the spec directory: `test-inventory-before.txt`
  (names from `cargo test --workspace -- --list`, default features, sorted, captured in US-0078) and
  `removed-tests.txt` (the removal list, grown by US-0079 and US-0080). The check, run in the Definition of done
  of US-0079 to US-0082, is: `after = before − removed` once names beginning `spec_0051_` are ignored. **Obliged
  by:** SPEC-0051/FR-NEW-003, E2E-002, E2E-020. **Rationale:** a runnable proof that no other test disappears.
  **Alternatives considered:** counting tests (hides swaps). **Source:** spec 14.3. **Evidence:** n/a
- **DEC-007:** Build: delete the `sqlserver` feature from both crates, set `all-backends = ["postgres", "rag"]`,
  delete the three optional dependencies and their workspace entries after `cargo tree -i tokio-util` confirms no
  other user, regenerate `Cargo.lock` with cargo. **Obliged by:** SPEC-0051/FR-DEL-002. **Rationale:** the spec
  keeps `all-backends`. **Alternatives considered:** dropping `all-backends` (breaks nothing in the repo but the
  spec keeps it). **Source:** spec. **Evidence:** `crates/core/Cargo.toml:19-27`, `crates/mcp-fs/Cargo.toml:15-20`,
  `Cargo.toml:43-56`
- **DEC-008:** The storage factories keep only `sqlite` and `postgres`; the `cfg(any(postgres, sqlserver))`
  gates become `cfg(postgres)`. **Obliged by:** SPEC-0051/FR-DEL-001. **Rationale:** no other user of the
  SQL Server gates. **Alternatives considered:** none. **Source:** existing code. **Evidence:**
  `crates/core/src/storage/mod.rs:45-76`, `149-185`
- **DEC-009:** `Dialect::SqlServer` and every arm on it are deleted; the SPEC-0050 helpers `char_len` and
  `exact_equals` and the SQL Server branch of `substr_fn` go, `SubtreeMatch` uses `substr`, `chars().count()` and
  `=` directly. **Obliged by:** SPEC-0051/FR-DEL-001. **Rationale:** the simplification the change exists for.
  **Alternatives considered:** keeping arms without a driver (untested dead code). **Source:** existing code.
  **Evidence:** `crates/core/src/storage/rel/dialect.rs:13-17`, `233-240`, `257-300`
- **DEC-010:** `ColumnType::TextKey(n)` is folded into `ColumnType::Text` and its `*_LEN` constants removed.
  **Obliged by:** SPEC-0051/FR-NEW-003 (layout unchanged), FR-DEL-001. **Rationale:** it exists for SQL Server
  index limits and renders `TEXT` on SQLite and PostgreSQL; the golden proves no definition moves. **Alternatives
  considered:** keeping the type (a no-op bound). **Source:** existing code. **Evidence:**
  `crates/core/src/storage/rel/dialect.rs:28`, `218`, `226`
- **DEC-011:** The SQL Server path limit goes: `max_path_len`, `ensure_path_fits`, `ensure_storable_path` and the
  `SafetyManager::new` parameter. The limits that apply everywhere stay with their messages: branch name 255 bytes
  and token 8 192 characters, only their comments are corrected. **Obliged by:** SPEC-0051/FR-DEL-001,
  FR-NEW-003. **Rationale:** `max_path_len` is `None` on every remaining backend. **Alternatives considered:**
  keeping the parameter as `Option` always `None` (dead surface). **Source:** existing code. **Evidence:**
  `crates/core/src/safety.rs:101-115`, `crates/core/src/tools/git.rs:2195`, `crates/core/src/tools/git_auth.rs:27`
- **DEC-012:** The tests naming SQL Server are deleted with their subject; shared tests keep their PostgreSQL and
  SQLite assertions; the conformance `Engine::SqlServer` goes. **Obliged by:** SPEC-0051/FR-DEL-001, E2E-020.
  **Rationale:** DEC-006 proves nothing else disappears. **Alternatives considered:** `#[ignore]` (keeps dead code).
  **Source:** spec. **Evidence:** `crates/core/src/storage/conformance.rs:43-95`
- **DEC-013:** Documentation: every file under `README.md`, `AGENTS.md`, `.agent_docs/` and the compose comments
  drops SQL Server; the README gets exactly the sentence `SQL Server support was removed on 2026-10-23
  (SPEC-0051).`; `lineage.md` may keep dated lines. **Obliged by:** SPEC-0051/FR-NEW-004. **Rationale:** the
  spec fixes the sentence. **Alternatives considered:** a longer migration note. **Source:** spec.
  **Evidence:** `README.md:9`, `.agent_docs/backends.md:149-218`
- **DEC-014:** Acceptance tests that need the process boundary spawn the real binary through
  `env!("CARGO_BIN_EXE_mcp-fs")` in `crates/mcp-fs/tests/sql_server_removed.rs`; structural checks (lock, compose,
  docs, cargo feature) live in the same file. **Obliged by:** SPEC-0051/all E2E. **Rationale:** stdout, stderr and
  exit status are the observables; the pattern of `security_gate.rs` already runs commands from tests.
  **Alternatives considered:** calling `ServerConfig::from_yaml` only (misses the channel and the exit status).
  **Source:** existing code. **Evidence:** `crates/mcp-fs/tests/security_gate.rs`

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0078 | refuse SQL Server in the configuration, with baselines | P1 | FR-NEW-001, 002, FR-MOD-001, FR-NEW-003, FR-DEL-001 | main | 2 | no |
| US-0079 | remove the driver, the build option and the test service | P1 | FR-DEL-002, FR-DEL-001, FR-NEW-003 | US-0078 | 2 | no |
| US-0080 | collapse the dialect and the bounded keys | P2 | FR-DEL-001, FR-NEW-003 | US-0079 | 2 | no |
| US-0081 | remove the path length limit | P2 | FR-DEL-001, FR-NEW-003 | US-0080 | 2 | no |
| US-0082 | documentation and spec close | P2 | FR-NEW-004, FR-NEW-003 | US-0081 | 2 | yes |
