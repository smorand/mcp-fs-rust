# Remove SQL Server support: functional specification

> Id: SPEC-0051
> Nature: FEAT
> Depth: L
> Depth evidence: FEAT / depth L: 5 areas touched (storage, configuration and startup, the data migration command, build, test environment and documentation), a removal of a supported database, 7 requirements.
> Status: designed
> Generated on: 2026-10-23
> Project: existing
> From backlog: BL-0073 (becomes void)
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary

The product can store its state in SQLite, PostgreSQL or SQL Server. SQL Server was added for one client, who has since rewritten its own version elsewhere, so nobody uses it and it costs code, tests, a slow test service and a young database driver. This increment removes SQL Server as a supported database. SQLite and PostgreSQL keep behaving exactly as before; an operator whose configuration still names SQL Server gets a clear refusal at startup instead of a silent surprise.

## 2. Current State

### 2.1 What users can do today

- An operator chooses the database of each of four stores (metadata, admin, git, oauth) among `sqlite`, `postgres` and `sqlserver`. SQL Server needs a connection string and a binary built with its option; without them startup is refused with a message naming the missing piece.
- An unknown database name is refused at startup with `unknown infra.<section>.backend '<name>', expected one of sqlite, postgres, sqlserver`.
- The data migration command copies state between any two supported databases, SQL Server included.
- A path longer than a SQL Server column is refused with a dedicated message when the metadata store is SQL Server; the other databases never refuse a path for its length.
- A branch name above 255 bytes and a seeded token above 8 192 characters are refused whatever the database.
- A developer can start a SQL Server container with the test environment and run an opt in SQL Server suite.
- Search is not available on SQL Server (it answers that the feature is not supported).
- The documentation presents SQL Server as the third supported database.

### 2.2 Specs governing this area

- SPEC-0023 (SQL Server metadata store cannot be created, implemented): its fix and its tests disappear with the database. Not modified.
- SPEC-0050 (case sensitive folder operations, implemented): the SQL Server rendering it added (binary comparison, UTF-16 length) disappears; its PostgreSQL and SQLite behavior is unchanged.
- BL-0073 (unverified: paths differing only by case on SQL Server) becomes void and is deleted with this spec.
- SPEC-0014 and the git specs: the 255 byte branch name limit (git SPEC FR-NEW-116) and the token bound stay as they are.

### 2.3 What is verified today

The SQL Server specific tests are about 49 test functions that mention SQL Server across 9 files (recounted in Section 14.1); most of them are opt in. The default suite runs on SQLite; the PostgreSQL suites are opt in.

## 3. Scope

### 3.1 In scope

- A configuration naming SQL Server is refused at startup with an explicit message.
- The list of accepted databases in the unknown name message becomes `sqlite, postgres`.
- The migration command refuses a SQL Server side with the same message.
- The SQL Server build option, its test service and its documentation are gone.
- SQLite and PostgreSQL behavior, stored layout and limits are unchanged.

### 3.2 Out of scope, and why

| Non goal | Reason | Backlog |
|---|---|---|
| Relaxing the branch name (255 bytes) or token (8 192) limits | they apply on every database and are observable; changing them is another change | none, decided |
| Migrating data away from SQL Server | the client runs its own version elsewhere; nothing deployed | none, decided |
| Removing SQLite | it is the default with no external service | none, decided |
| Keeping an alias for the SQL Server build option | nobody builds with it | none, decided |

### 3.3 Constraints imposed by the context

- Imposed by the project owner: SQLite stays; the explicit refusal message stays; the migration command stays between SQLite and PostgreSQL.
- Imposed by the conventions: the default build and the default test run need no database and no container.

## 4. Actors

| Actor | Description |
|---|---|
| Operator | Writes the configuration and starts the server or the migration command. |
| Builder | Builds the binary and chooses its build options. |
| Developer | Runs the test suites and the test environment. |
| Maintainer | Reads the documentation. |

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Storage | the supported databases and the stored layout | SQLite, PostgreSQL, store, backend |
| Operations | configuration, startup and data migration | configuration section, startup refusal, migration |
| Delivery | build options, test environment, documentation | build option, test service, documentation |

## 5. Usage scenarios

### SC-001: An operator keeps running on SQLite or PostgreSQL
**Actor:** Operator
**Preconditions:** a configuration naming `sqlite` or `postgres` for every store.
**Flow:**
1. The operator starts the server → it starts, answers its health check and serves exactly what it served before.
**Postconditions:** the stored data written by the previous version is read without any migration.
**Exceptions:** none new.

### SC-002: An operator still names SQL Server
**Actor:** Operator
**Preconditions:** one of the stores (`meta`, `admin`, `git`, `oauth`) is configured with the database `sqlserver`, with or without a connection string.
**Flow:**
1. The operator starts the server → it refuses to start and prints `infra.<section>.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`.
**Postconditions:** nothing was opened, created or written.
**Exceptions:**
- EXC-002a: the name is spelled otherwise (`mssql`, `oracle`) → `unknown infra.<section>.backend '<name>', expected one of sqlite, postgres`.

### SC-003: The operator migrates between supported databases
**Actor:** Operator
**Preconditions:** a source configuration and a destination configuration.
**Flow:**
1. The operator runs the migration command from SQLite to PostgreSQL → the state is copied as before.
**Postconditions:** the destination holds what the source held.
**Exceptions:**
- EXC-003a: either side names SQL Server → refused with the same message as SC-002, before any data is read or written.

### SC-004: A builder or a developer uses the project
**Actor:** Builder, developer
**Preconditions:** a checkout of the project.
**Flow:**
1. The builder asks for the SQL Server build option → the build tool refuses it as unknown.
2. The developer starts the test environment → it offers PostgreSQL only.
**Postconditions:** the dependency lock holds no SQL Server driver and no connection pool made for it.

### SC-005: A maintainer reads the documentation
**Actor:** Maintainer
**Preconditions:** the repository documentation.
**Flow:**
1. The maintainer looks for the supported databases → SQLite and PostgreSQL are the two listed; SQL Server is described only as removed.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-O]: A configuration naming SQL Server is refused
> IF a store section of the configuration names the database `sqlserver` THEN THE Operations context SHALL refuse to start and print `infra.<section>.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`.
- **Business rules:** the four store sections are `meta`, `admin`, `git` and `oauth`; `<section>` is the first one naming `sqlserver`; the removal refusal is decided before any other check of the configuration: all four sections are scanned in the order meta, admin, git, oauth, and the first naming `sqlserver` is refused even if an earlier section or another part of the configuration is also invalid; only when none names `sqlserver` do the other checks run, in their existing order; the refusal is decided on the configuration once it is read as a structured document and `${VAR}` placeholders are expanded, so a file that cannot be read as such a document (not found, invalid YAML, a wrongly typed value anywhere) keeps today's error (`invalid config: ...`, `configuration file not found: ...` or `no configuration file found, tried: ...`) even if a store names `sqlserver`, while any other validation failure, `safety.max_diff_file_bytes` included, comes after the removal refusal; the refusal is given whether or not a connection string is present and before any complaint about it; the connection string is never printed; the process opens or writes nothing. A refusal is written to the standard error as its last line, in the form every startup refusal has today: `error: ERR_INVALID_ARGUMENT: <message>`; the standard output stays empty and the exit status is 1.
- **Scenario:** SC-002 **Priority:** Must

#### FR-NEW-002 [EARS-O]: The migration command refuses SQL Server
> IF either configuration given to the migration command names `sqlserver` for a store THEN THE Operations context SHALL refuse with the message of FR-NEW-001 before reading or writing any data.
- **Business rules:** the source configuration is checked before the destination; the first offending section is named, in the order meta, admin, git, oauth; the two configurations are loaded and checked one after the other, source first, each as in FR-NEW-001, so an unrelated error in the source is reported before a SQL Server refusal in the destination; on a refusal neither the line `migrating <source> -> <destination>` nor a row total is printed, and the exit status is 1.
- **Scenario:** SC-003 **Priority:** Must

#### FR-NEW-003 [EARS-U]: SQLite and PostgreSQL are unchanged
> The Storage context SHALL keep, on SQLite and on PostgreSQL, every answer, every limit and the stored layout it had before this change.
- **Business rules:** a database written by the previous version opens without any migration; the limit of 255 bytes on a branch name, of 8 192 characters on a seeded token, and the absence of a limit on path length are unchanged; the layout of the stored tables is byte identical; the search tools answer on SQLite and PostgreSQL exactly as before.
- **Scenario:** SC-001 **Priority:** Must

#### FR-NEW-004 [EARS-U]: The documentation lists two databases
> The Delivery context SHALL document SQLite and PostgreSQL as the supported databases, and SHALL describe SQL Server only as removed.
- **Business rules:** every file of the README, the agent index, the reference documents and the comments of the test environment definition presents no SQL Server instruction, driver, example, test command or environment variable; the removal note is exactly one sentence in the README, `SQL Server support was removed on 2026-10-23 (SPEC-0051).`, and the same sentence is allowed in the history document; the history document may also keep any line that carries a date. The configuration example in the README lists `sqlite | postgres`.
- **Scenario:** SC-005 **Priority:** Should

### 6.2 Modified

#### FR-MOD-001 [EARS-E]: The unknown database message lists two names
> WHEN a store section names a database that is neither `sqlite`, `postgres` nor `sqlserver` THE Operations context SHALL refuse to start and print `unknown infra.<section>.backend '<name>', expected one of sqlite, postgres`.
- **Before:** the message ended `expected one of sqlite, postgres, sqlserver`.
- **After:** the message ends `expected one of sqlite, postgres`.
- **Reason:** SQL Server is no longer a supported name.
- **Scenario:** SC-002 **Priority:** Must

### 6.3 Removed

#### FR-DEL-001: SQL Server as a database
- **What the user loses:** the choice of SQL Server for any of the four stores; the dedicated refusal of a path longer than a SQL Server column (`path is <N> characters but this backend (sqlserver) allows at most <M>: shorten a directory name or move the entry nearer the volume root, ...`), which can no longer be produced; the migration to or from SQL Server.
- **Reason:** unused since the client moved to its own implementation; costs a driver, a pool, a dialect and slow tests.
- **What happens to existing data or habits:** nothing is deployed on it; a configuration naming it gets the refusal of FR-NEW-001.
- **Scenario:** SC-002, SC-003

#### FR-DEL-002: The SQL Server build option and test service
- **What the user loses:** the build option `sqlserver`, the SQL Server container of the test environment and the SQL Server opt in suite; the build option `all-backends` stays and now means PostgreSQL and vector search.
- **Reason:** nothing to build or test.
- **What happens to existing data or habits:** asking cargo for `--features sqlserver` fails with cargo's unknown feature error; the dependency lock no longer holds the SQL Server driver or the pool made for it.
- **Scenario:** SC-004

## 7. Non functional requirements (user observable)

### 7.1 Performance
The default test run (SQLite, no database, no container) is not slower than before; the compile time of the default build does not grow.

### 7.2 Security and access
The refusal message never contains the connection string or any part of it.

### 7.5 Reliability
A refused configuration changes nothing on disk or in any database.

### 7.6 Operability
The refusal names the section and the two supported databases, so an operator knows exactly what to change.

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | `sqlserver` without a connection string | the removal message, not a connection string complaint | E2E-008 |
| Boundary values | branch name 255 and 256 bytes; token 8 192 and 8 193 characters | accepted, refused, as before | E2E-017, E2E-018 |
| Concurrent actions | not applicable: configuration is read once at startup | n/a | n/a |
| Slow or lost connectivity | not applicable: the refusal happens before any connection | n/a | n/a |
| Access states | not applicable: no access rule changes | n/a | n/a |
| Data variations | spellings `mssql`, `oracle`; a connection string with a password; a 2 000 character path | unknown name message; password not printed; path accepted | E2E-009, E2E-014, E2E-019 |
| Navigation | two sections naming SQL Server | the first in the order meta, admin, git, oauth | E2E-013 |
| Partial operations | migration with a SQL Server side | nothing read or written | E2E-010, E2E-011 |
| Dependency failures | a build asked for the removed option | the build tool refuses | E2E-012 |
| Ordering and timing | not applicable | n/a | n/a |
| Exhaustion | not applicable | n/a | n/a |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. /sdlc-design maps each test to a driver.

A refusal means: the standard error ends with the line `error: ERR_INVALID_ARGUMENT: <message>`, the standard output is empty and the exit status is 1.

Shared setup unless stated: a configuration whose four stores use `sqlite`, a state directory holding project `Atlas` with 3 files, and for the PostgreSQL tests the PostgreSQL service of the test environment.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-003 | Critical |
| E2E-002 | New | happy | SC-001 | FR-NEW-003 | Critical |
| E2E-003 | New | happy | SC-003 | FR-NEW-003 | High |
| E2E-004 | New | happy | SC-005 | FR-NEW-004 | Medium |
| E2E-005 | New | failure | SC-002 | FR-NEW-001, FR-DEL-001 | Critical |
| E2E-006 | New | failure | SC-002 | FR-NEW-001 | Critical |
| E2E-007 | New | failure | SC-002 | FR-NEW-001 | Critical |
| E2E-008 | New | failure | SC-002 | FR-NEW-001 | High |
| E2E-009 | New | failure | SC-002 | FR-MOD-001 | High |
| E2E-010 | New | failure | SC-003 | FR-NEW-002, FR-DEL-001 | High |
| E2E-011 | New | failure | SC-003 | FR-NEW-002 | High |
| E2E-012 | New | failure | SC-004 | FR-DEL-002 | High |
| E2E-013 | New | failure | SC-002 | FR-NEW-001 | High |
| E2E-014 | New | side effect | SC-002 | FR-NEW-001 | Critical |
| E2E-015 | New | side effect | SC-004 | FR-DEL-002 | Medium |
| E2E-016 | New | invariance | SC-001 | FR-NEW-003 | Critical |
| E2E-017 | New | invariance | SC-001 | FR-NEW-003 | High |
| E2E-018 | New | invariance | SC-001 | FR-NEW-003 | High |
| E2E-019 | New | invariance | SC-001 | FR-NEW-003 | High |
| E2E-020 | New | regression | SC-001 | FR-NEW-003 | Critical |
| E2E-021 | New | failure | SC-002 | FR-NEW-001, FR-MOD-001 | High |
| E2E-022 | New | failure | SC-002 | FR-NEW-001 | High |

Coverage: happy 4, failure 11, side effect 2, invariance 4, regression 1, state transition 0, security 0. Happy to failure ratio 1:2.75 (must beat 1:1).

### 9.2 New tests

#### E2E-001: A SQLite server starts and answers
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** Critical
- **Given** the shared setup
- **When** the operator starts the server
- **Then** the health check answers ok and a member of `Atlas` lists its 3 files

#### E2E-002: The PostgreSQL suite passes unchanged
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** Critical
- **Given** the PostgreSQL service of the test environment is up
- **When** the developer runs the PostgreSQL end to end suite
- **Then** every test passes, and the set of test names of the suite equals the inventory recorded before this change (Section 14.3, b) minus exactly the tests of the removal list

#### E2E-003: The migration from SQLite to PostgreSQL still copies the state
- **Category:** happy. **Scenario:** SC-003. **Requirements:** FR-NEW-003. **Priority:** High
- **Given** the shared setup and an empty PostgreSQL database
- **When** the operator migrates the SQLite configuration to the PostgreSQL configuration
- **Then** `Atlas` exists on PostgreSQL with the same 3 files and the same sizes

#### E2E-004: The documentation lists two databases
- **Category:** happy. **Scenario:** SC-005. **Requirements:** FR-NEW-004. **Priority:** Medium
- **Given** the README, the agent index (`AGENTS.md`), every reference document under the agent documents folder and the comments of the test environment definition
- **When** the maintainer searches them for `sqlserver`, `SQL Server`, `mssql`, `tiberius` and `bb8`
- **Then** every hit of `sqlserver`, `SQL Server`, `mssql`, `tiberius` or `bb8` is the removal sentence `SQL Server support was removed on 2026-10-23 (SPEC-0051).`, the refusal message of FR-NEW-001, or a dated line of the history document; none is an instruction, an example, a driver mention or a test command (the scan excludes `sdlc/`, `plan/` and `Cargo.lock`)

#### E2E-005: The metadata store cannot be SQL Server
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001, FR-DEL-001. **Priority:** Critical
- **Given** a configuration whose `meta` store names `sqlserver` with the connection string `server=h;database=d`
- **When** the operator starts the server
- **Then** it is refused with `ERR_INVALID_ARGUMENT: infra.meta.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`
- **And** no state directory, database file or connection was created

#### E2E-006: The admin store cannot be SQL Server
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001. **Priority:** Critical
- **Given** a configuration whose `admin` store names `sqlserver` and the other stores `sqlite`
- **When** the operator starts the server
- **Then** it is refused with `ERR_INVALID_ARGUMENT: infra.admin.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`

#### E2E-007: The oauth store cannot be SQL Server
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001. **Priority:** Critical
- **Given** a configuration whose `oauth` store names `sqlserver` and the other stores `sqlite`
- **When** the operator starts the server
- **Then** it is refused with `ERR_INVALID_ARGUMENT: infra.oauth.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`

#### E2E-008: Without a connection string the removal message still comes first
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001. **Priority:** High
- **Given** a configuration whose `meta` store names `sqlserver` with no connection string
- **When** the operator starts the server
- **Then** the message is the one of E2E-005, and no message about a missing connection string appears

#### E2E-009: Other unknown names list two databases
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-MOD-001. **Priority:** High
- **Given** a configuration whose `meta` store names `mssql`, then one naming `oracle`
- **When** the operator starts the server each time
- **Then** the refusals are `ERR_INVALID_ARGUMENT: unknown infra.meta.backend 'mssql', expected one of sqlite, postgres` and `ERR_INVALID_ARGUMENT: unknown infra.meta.backend 'oracle', expected one of sqlite, postgres`

#### E2E-010: A migration from SQL Server is refused
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-002, FR-DEL-001. **Priority:** High
- **Given** a source configuration whose `meta` store names `sqlserver` and an empty destination PostgreSQL database
- **When** the operator runs the migration command
- **Then** it is refused with the message of E2E-005
- **And** the destination database holds no table written by the migration, the standard output is empty and the standard error does not contain `migrating`

#### E2E-011: A migration to SQL Server is refused
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-002. **Priority:** High
- **Given** the shared setup as the source and a destination configuration whose `meta` store names `sqlserver`
- **When** the operator runs the migration command
- **Then** it is refused with the message of E2E-005
- **And** the SQLite source is unchanged (still 3 files in `Atlas`), the standard output is empty and the standard error does not contain `migrating`

#### E2E-012: The removed build option is refused
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-DEL-002. **Priority:** High
- **Given** a checkout of the project
- **When** the builder runs `cargo build --features sqlserver`
- **Then** cargo exits with a non zero status and says the feature `sqlserver` is not found
- **And** `cargo build --features all-backends` still resolves its features

#### E2E-013: With several sections naming SQL Server the first is named
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001. **Priority:** High
- **Given** a configuration whose `git` and `oauth` stores name `sqlserver`, and one whose `meta` and `oauth` stores do
- **When** the operator starts the server each time
- **Then** the first refusal names `infra.git.backend` and the second names `infra.meta.backend`

#### E2E-014: The connection string is never printed
- **Category:** side effect. **Scenario:** SC-002. **Requirements:** FR-NEW-001. **Priority:** Critical
- **Given** a configuration whose `meta` store names `sqlserver` with the connection string `Server=h;Password=Secret1;Database=d`
- **When** the operator starts the server
- **Then** neither the standard output nor the standard error contains `Secret1`, `Server=h` or `Database=d`

#### E2E-015: The test environment no longer offers SQL Server
- **Category:** side effect. **Scenario:** SC-004. **Requirements:** FR-DEL-002. **Priority:** Medium
- **Given** the test environment definition
- **When** the developer lists its services
- **Then** the list is `postgres` only and no service is named `mssql`
- **And** the dependency lock holds no package named `tiberius-ng`, `tiberius-ng-macros`, `tiberius` or `bb8` (or any package whose name starts with `tiberius`)

#### E2E-016: The stored layout is unchanged
- **Category:** invariance. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** Critical
- **Given** the table definitions SQLite and PostgreSQL produce for the four stores, captured from the previous version
- **When** the same definitions are produced after this change
- **Then** they are byte identical for SQLite and for PostgreSQL
- **And** a SQLite database written by the previous version opens and lists `Atlas` with its 3 files

#### E2E-017: The branch name limit is unchanged
- **Category:** invariance. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** High
- **Given** a project with git enabled
- **When** a member creates a branch named with 255 bytes, then one with 256 bytes
- **Then** the first is accepted and the second is refused with `'<name>' is not a valid branch name: it is 256 bytes of UTF-8, over the 255 byte limit` (with the 256 byte name in place of `<name>`), the same message as before this change

#### E2E-018: The token limit is unchanged
- **Category:** invariance. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** High
- **Given** git enabled and a member seeding a token
- **When** the member seeds a token of 8 192 characters, then one of 8 193
- **Then** the first is accepted and the second is refused with `token must be at most 8192 characters`, the same message as before this change

#### E2E-019: A very long path is accepted on SQLite
- **Category:** invariance. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** High
- **Given** the shared setup
- **When** a member writes a file under a path of 2 000 characters
- **Then** the write is accepted and the file reads back with its content

#### E2E-020: The existing suite passes
- **Category:** regression. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** Critical
- **Given** the repository after this change
- **When** the developer runs the full quality gate
- **Then** it passes; the set of test names equals the inventory recorded before this change minus exactly the removal list, so no other test is gone; and the search tool tests pass on SQLite

#### E2E-021: The removal refusal comes before any other configuration error
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001, FR-MOD-001. **Priority:** High
- **Given** a configuration whose `meta` store names `postgres` with no connection string and whose `admin` store names `sqlserver`; and one whose `meta` store names `mssql` and whose `admin` store names `sqlserver`
- **When** the operator starts the server each time
- **Then** both refusals are `ERR_INVALID_ARGUMENT: infra.admin.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`

#### E2E-022: A file that cannot be read keeps its own error, other validation comes after the removal
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-001. **Priority:** High
- **Given** a configuration where `safety.max_diff_file_bytes` is 0 and the `admin` store names `sqlserver`; and a configuration file that is not valid YAML and also names `sqlserver`
- **When** the operator starts the server each time
- **Then** the first refusal is `ERR_INVALID_ARGUMENT: infra.admin.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'`; the second starts with `error: ERR_INVALID_ARGUMENT: invalid config: ` as today

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-003 | E2E-001, 002 | none (an invariance scenario) | E2E-016 to 020 |
| SC-002 | FR-NEW-001, FR-MOD-001, FR-DEL-001 | none (a refusal scenario) | E2E-005 to 009, 013, 021, 022 | E2E-008, 013, 014 |
| SC-003 | FR-NEW-002, FR-DEL-001 | E2E-003 | E2E-010, 011 | E2E-010, 011 |
| SC-004 | FR-DEL-002 | none (a delivery scenario) | E2E-012 | E2E-015 |
| SC-005 | FR-NEW-004 | E2E-004 | none (a documentation scenario) | E2E-004 |

| Requirement | Tests |
|---|---|
| FR-NEW-001 | E2E-005, 006, 007, 008, 013, 014, 021, 022 |
| FR-NEW-002 | E2E-010, 011 |
| FR-NEW-003 | E2E-001, 002, 003, 016, 017, 018, 019, 020 |
| FR-NEW-004 | E2E-004 |
| FR-MOD-001 | E2E-009, 021 |
| FR-DEL-001 | E2E-005, 010 |
| FR-DEL-002 | E2E-012, 015 |

## 11. Open questions

None.

## 12. Glossary

| Term | Definition | Context |
|---|---|---|
| Store | one of the four places state is kept: metadata, admin, git, oauth | Storage |
| Section | the configuration block of a store: `meta`, `admin`, `git`, `oauth` | Operations |
| Backend | the database a store uses: `sqlite` or `postgres` | Storage |
| Refusal | the startup failure with a message and a non zero exit status | Operations |
| Migration | the command copying state from one configuration to another | Operations |
| Build option | a choice made when compiling the binary | Delivery |
| Test environment | the container definition used by the opt in test suites | Delivery |

## 13. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 3 | 5 | NOT-IMPLEMENTABLE |
| 2 | 2 | 4 | NOT-IMPLEMENTABLE |
| 3 | 1 | 4 | NOT-IMPLEMENTABLE |
| 4 | 0 | 5 | IMPLEMENTABLE |

**Amendments applied:** refusal form and channel (stderr, `error: ` prefix, exit status 1); migration order and silence; baselines for the schema, the test inventory and the lock; documentation scope and the exact removal sentence; precedence of the removal refusal and the carve out for unreadable files; E2E-021 and E2E-022; verbatim limit messages; the connection string never printed (no log file claim); lock check names (`tiberius-ng-macros`, `bb8`); the previous version fixture; traceability fixes.
**No implementation detail outside Section 14:** PASS

## 14. Evidence appendix

### 14.1 Current State evidence

| Claim (Section 2) | Evidence |
|---|---|
| Accepted database names and the unknown name message | `crates/core/src/config.rs:14-19`, `crates/core/src/config.rs:1076-1079` |
| SQL Server needs a dsn and its build feature | `crates/core/src/config.rs:1062-1073` |
| Four stores validated in the order meta, admin, git, oauth | `crates/core/src/config.rs:976-979` |
| Configuration errors print `ERR_INVALID_ARGUMENT: <message>` | `crates/core/src/errors.rs:159-162`, `crates/core/src/cli.rs:131-135` |
| The migration command loads both configurations through the same validation | `crates/core/src/cli.rs:168-181` |
| Path length refusal exists only for SQL Server | `crates/core/src/safety.rs:101-115` |
| Branch name limit 255 bytes | `crates/core/src/tools/git.rs:2195` |
| Token bound 8 192 | `crates/core/src/tools/git_auth.rs:27` |
| SQL Server driver file | `crates/core/src/storage/rel/sqlserver.rs` (774 lines) |
| Build options | `crates/core/Cargo.toml:19-27`, `crates/mcp-fs/Cargo.toml:18`, `Cargo.toml:43-56` |
| Test service | `docker-compose.test.yml:39-54` |
| About 49 hits in test functions (a function counts when its body names SQL Server) across 9 test bearing files; 17 source files mention SQL Server in all, the rest being production code or comments to clean (`crates/core/src/tools/git.rs:2192`, `crates/core/src/tools/git_auth.rs:25`, `crates/core/src/migrate.rs:44`, `crates/core/src/git/oauth/persistence.rs`, `crates/core/src/git/db.rs`; also fix the stale crate path in the `MAX_TOKEN_LEN` comment) | counted with a script over `rg -il 'sqlserver|mssql|sql server|tiberius' crates --glob '*.rs'`: dialect.rs 16, sqlserver.rs 10, safety.rs 5, meta.rs 5, schema.rs 4, teardown_e2e.rs 4, config.rs 3, mssql_trash_e2e.rs 1, conformance.rs 1 (a function counts when its body names SQL Server; recount in design) |
| Documentation mentions (approximate, recount in design; `lib.rs:2`, `tools/mod.rs:64` and the compose header also name SQL Server) | `rg -c -i 'sql ?server|mssql|sqlserver'`: README.md 9, AGENTS.md 10, `.agent_docs/backends.md` 20, `testing.md` 8, `architecture.md` 7, `config.md` 5, `search.md` 2, `lineage.md` 2, `git.md` 1 |

Test command: `make check`.

### 14.3 Design inputs

- Capabilities absent today, to be created BEFORE any code change and committed first: (a) a schema golden: the table definitions of the four stores on SQLite and on PostgreSQL written to a committed file; a test compares the definitions produced after the change byte for byte (detection: the test fails on any difference; E2E-016); also commit, before any code change, a SQLite state directory fixture written by the unmodified binary (project `Atlas`, 3 files) that E2E-016 and E2E-001 reopen (detection: the test fails if `Atlas` or its 3 files are missing); (b) a test inventory: the list of test names of the workspace (`cargo test --workspace -- --list`) recorded before the change; after it, the set difference must equal exactly the removal list (the tests naming SQL Server, recounted per function) (detection: a check that fails on any other missing name; E2E-002, E2E-020); (c) a lock check: the dependency lock has no entry for `tiberius-ng`, `tiberius-ng-macros`, `tiberius` or `bb8` (or any package whose name starts with `tiberius`) (detection: a text search of the lock; E2E-015).

- The dialect enum, the `match` arms on it, the schema rendering and `max_path_len` shrink with the removal; `TextKey` (bounded keys) exists only for SQL Server index limits and renders plain text elsewhere (`crates/core/src/storage/meta.rs:93-224` including the path key size, `crates/core/src/git/db.rs`, the token bound note at `crates/core/src/git/oauth/persistence.rs:418`). Renaming it is a design choice; the stored layout must stay byte identical. Signal: E2E-016 (a golden of the table definitions captured BEFORE any change).
- The ref name limit of 255 bytes and the token bound exist because of `TextKey`; they must stay (FR-NEW-003). Signals: E2E-017, E2E-018.
- The build gates `cfg(any(postgres, sqlserver))` become `cfg(postgres)` (`crates/core/src/storage/mod.rs:45-76`); `tokio-util` is optional only for the SQL Server driver (`crates/core/Cargo.toml:22`, `47`; workspace definition `Cargo.toml:56`, `bb8` at `Cargo.toml:53`): confirm with `cargo tree -i tokio-util` before removing it. Signal: the build and E2E-015.
- `--all-features` in the Makefile (`Makefile:23-31`) keeps working once the feature is gone. Signal: E2E-020.
- The migration batch size derives from SQL Server's 2 100 parameter limit (`crates/core/src/migrate.rs:46`); changing it must not alter results. Signal: E2E-003.
- The documentation of a SQL Server "ERR_NOT_SUPPORTED" search path (`.agent_docs/search.md:185-187`) and the driver decision record (`.agent_docs/backends.md:149-218`) go; one dated note stays. Signal: E2E-004.
- SPEC-0023's tests and SPEC-0050's SQL Server rendering tests are removed with the database; the case collision conformance case stays for SQLite and PostgreSQL. Signal: E2E-020.
