# mcp-fs Optional Integrations — Specification Document

> Generated on: 2026-09-18
> Project: mcp-fs (Rust)
> Version: 1.0
> Status: Draft
> Type: Evolution Specification (retro-specification of shipped behaviour)

**ID convention.** S8 of eight. S8 owns the `8xx` block: `SC-8xx`, `FR-8xx`, `E2E-8xx`, `DEC-8xx`, `EXC-8xx`.

## 1. Executive Summary

This document specifies the four optional tool families that extend an agent's reach beyond the filesystem: **`web.*`** (5 tools, keyless DuckDuckGo search and page retrieval), **`context7.*`** (2 tools, library documentation lookup), **`sqlite.*`** (8 tools, persistent SQLite databases stored as volume files), and **`db.*`** (5 tools, DataFusion analytics over CSV, Parquet and JSON).

They share one shape. Each is off by default and registered only when its config flag is set. Each reaches something the filesystem does not hold: the public web, a documentation API, a query engine. And each, when it writes back into a volume, does so through `core::fs_ops` so that the ACL, the write quota and the audit log apply exactly as they do to `fs.write`. An integration is a new way to produce content, never a way around the rules that govern content.

Two share a further pattern worth naming: `sqlite.*` and `db.*` both implement **extract-operate-commit**. The stored file is materialized to a temporary path because the underlying engine needs a real POSIX path, the operation runs there, and any result is written back through the engine.

Retro-specification of shipped behaviour; every claim carries a `file:LINE` citation.

## 2. Current State Analysis

### 2.1 Project Overview

Four modules under `tools/`: `web.rs`, `context7.rs`, `sqlite.rs`, `db.rs`, each with a `register(reg, config)` entry point called only when its flag is set (`tools/all.rs:37-43`). Together they add 20 tools to the 45 always-on ones.

### 2.2 Existing Specifications

- **S1**: the membership gate, the error vocabulary, the config schema and the registration gating.
- **S2**: `core::fs_ops`, which every volume write in this document goes through, and the safety rules it enforces.
- **S4**: the relational seam. Note that `sqlite.*` does **not** use it: these are user databases stored as files, not server state.
- **S7**: indexing hooks that fire when these families write into a volume.

### 2.3 Relevant Architecture

- **Registration**: `tools/all.rs:37-43`, one branch per family.
- **Config**: `WebConfig` (`config.rs:486-491`), `Context7Config` (`config.rs:512-517`), `SqliteConfig` (`config.rs:531-537`), `DbConfig` (`config.rs:547-554`).
- **Locking**: a process-wide lock map keyed by `(mount_id, db_path)` (`tools/sqlite.rs:30-37`).
- **Statement gate**: `sqlite.query` accepts only `SELECT` or `WITH` (`tools/sqlite.rs:82-90`).
- **Table naming**: the file stem lowercased (`tools/db.rs:7-8,55-56`).

## 3. Scope

### 3.1 In Scope

- The 5 `web.*` tools: `search`, `news`, `suggestions`, `fetch`, `download`.
- The 2 `context7.*` tools: `resolve_library_id`, `get_library_docs`.
- The 8 `sqlite.*` tools: `query`, `execute`, `list_tables`, `describe_table`, `list_indexes`, `import_csv`, `export_csv`, `vacuum`.
- The 5 `db.*` tools: `query`, `schema`, `sample`, `profile`, `convert`.
- Registration gating and the config block of each family.
- The rules governing writes back into a volume.
- The extract-operate-commit pattern and its concurrency control.
- Result caps and size limits.

### 3.2 Out of Scope (Non-Goals)

- Everything S1 owns: authorization, the error vocabulary, config loading.
- The filesystem engine (S2). These families call it; they do not reimplement it.
- The relational seam (S4). The `sqlite.*` family operates on **user** databases stored as volume files, which is a different thing from the server's own relational state.
- Search indexing side effects (S7), which fire for these writes as for any other.
- Any guarantee about an external service's availability, content or stability. DuckDuckGo and Context7 are public services this server calls.

## 4. User Personas & Actors

| Actor | Description |
|---|---|
| **LLM agent** | The consumer. Searches the web for current information, looks up library documentation before writing code, queries a stored database, analyses a CSV. |
| **Project member** | The identity every volume write is attributed to, subject to the same quota and audit as any write. |
| **Operator** | Decides which families exist. Each is off by default, so enabling one is a deliberate choice about what the server reaches. |
| **External service** | DuckDuckGo and Context7. Both keyless, both public, both outside the deployment's control. |

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| **Web** | Public search and page retrieval. | query, result, page, download |
| **Documentation** | Library identity and its docs. | library id, docs |
| **User database** | SQLite databases stored as volume files. | db file, statement, table, index |
| **Analytics** | Columnar query over stored data files. | table name, schema, row, format |
| **Volume** (from S1) | Where anything written lands. | path, quota, audit entry |

A "database" in the User database context is a file in a volume that a caller owns; it is not the server's relational state, which S4 governs. A "table" in the Analytics context is a name derived from a file, not a persisted object.

## 5. Usage Scenarios

### SC-801: Agent searches the web and saves a page

**Actor:** LLM agent
**Preconditions:** `web.enabled` true.
**Flow:**
1. Agent calls `web.search` with a query; results carry title, URL and snippet.
2. Agent optionally calls `web.news` for recent items or `web.suggestions` for autocomplete.
3. Agent calls `web.fetch` on a promising URL.
4. When the page is large or its raw HTML matters, the agent passes `mount_id` and `save_path` so the content is written straight into a volume, bypassing the context window entirely.
5. For binary content the agent calls `web.download`, which always writes to a volume because binary data cannot pass through an LLM context.

**Postconditions:** the agent holds the results; any saved content is in the volume, charged against the session quota and audited.
**Exceptions:**
- EXC-801a: `mount_id` without `save_path`, or the reverse → `ERR_INVALID_ARGUMENT`, `"mount_id and save_path must both be provided, or neither"` (`tools/web.rs:84-87`)
- EXC-801b: writing to a project the caller does not belong to → `ERR_FORBIDDEN`
- EXC-801c: the write exceeds the session quota → `ERR_WRITE_QUOTA_EXCEEDED`
- EXC-801d: the external service unreachable or slow → the request times out per `request_timeout_secs`
- EXC-801e: more results requested than the cap → capped at 50

### SC-802: Agent looks up a library's documentation

**Actor:** LLM agent
**Preconditions:** `context7.enabled` true.
**Flow:**
1. Agent calls `context7.resolve_library_id` with a library name such as `tokio`.
2. Agent calls `context7.get_library_docs` with the resolved id.
3. The documentation comes back as Markdown, which the agent reads directly.

**Postconditions:** the agent holds current documentation rather than relying on its training data.
**Exceptions:**
- EXC-802a: an unknown library → an empty or unresolved result, not a server error
- EXC-802b: the API unreachable → a timeout after `request_timeout_secs`, default 30
- EXC-802c: calling `get_library_docs` with a name rather than a resolved id → whatever the API returns; resolution is a documented precondition, not an enforced one

### SC-803: Agent queries a SQLite database stored in a volume

**Actor:** LLM agent
**Preconditions:** `sqlite.enabled` true; the caller is a member; a `.db` file exists in the volume.
**Flow:**
1. Agent calls `sqlite.list_tables`, then `sqlite.describe_table` to learn the shape.
2. Agent calls `sqlite.query` with a `SELECT`.
3. The tool takes the per-database lock, reads the file's bytes, writes them to a temporary file because rusqlite needs a real POSIX path, opens the connection there, runs the statement, and drops the temporary file.
4. For a write the agent calls `sqlite.execute`; the modified bytes are read back and committed through `fs_ops::write_bytes`, so the quota, ACL and audit apply.
5. `sqlite.import_csv`, `sqlite.export_csv` and `sqlite.vacuum` follow the same pattern.

**Postconditions:** reads leave the file untouched; writes are committed with full accounting.
**Exceptions:**
- EXC-803a: a non-`SELECT` statement passed to `sqlite.query` → refused with `"sqlite.query only accepts SELECT or WITH statements; use sqlite.execute for writes"` (`tools/sqlite.rs:85-90`)
- EXC-803b: two concurrent requests against the same database → serialized by the lock, never concurrent (`tools/sqlite.rs:13`)
- EXC-803c: more rows than `max_result_rows` → capped
- EXC-803d: a statement exceeding `statement_timeout_secs` → aborted
- EXC-803e: the `.db` path is not a member's project → `ERR_FORBIDDEN`

### SC-804: Agent analyses a data file

**Actor:** LLM agent
**Preconditions:** `db.enabled` true; the caller is a member; a CSV, Parquet or JSON file exists.
**Flow:**
1. Agent calls `db.schema` to learn the columns, or `db.sample` for rows, or `db.profile` for statistics.
2. Agent calls `db.query` with SQL whose table name is the file stem lowercased: `/data/Sales_2024.csv` is queried as `FROM sales_2024`.
3. The file's bytes are written to a temporary file and registered with DataFusion.
4. `db.convert` writes a converted file back into the volume through the engine.

**Postconditions:** the agent holds the analysis; a conversion leaves a new file with full accounting.
**Exceptions:**
- EXC-804a: a file larger than `max_file_bytes`, default 100 MiB → refused
- EXC-804b: more rows than `max_result_rows` → capped, hard ceiling 10000
- EXC-804c: SQL naming a table that is not the file stem → a DataFusion error naming the unknown table
- EXC-804d: an unsupported format → refused

### SC-805: Operator chooses which integrations exist

**Actor:** Operator
**Preconditions:** a running deployment.
**Flow:**
1. Operator leaves every family disabled by default.
2. Operator enables a family in YAML, or forces it on with a CLI flag.
3. The tools appear in the catalogue; the others remain absent.

**Postconditions:** the server reaches exactly what the operator chose.
**Exceptions:**
- EXC-805a: a disabled family's tool called → the JSON-RPC unknown-tool error, because the tool is absent rather than present-and-failing
- EXC-805b: a CLI flag → forces the family on regardless of the YAML value (S1 FR-004)

## 6. Functional Requirements

### Common rules

#### FR-801 [EARS-O]: Each optional family is off by default and gated by its flag
> IF an optional family's `enabled` flag is false THEN none of its tools SHALL be registered.

- **Inputs:** `web.enabled`, `context7.enabled`, `sqlite.enabled`, `db.enabled`, and the matching CLI overrides.
- **Outputs:** the registered tool set (`tools/all.rs:37-43`).
- **Business Rules:** all four default to false (`config.rs:494,507,540,556`). The counts are 5 web, 2 context7, 8 sqlite, 5 db, verified by the registry tests in S1. An LLM must not see a tool it cannot call, which is why the tools are absent rather than registered and failing.
- **Priority:** Must-have

#### FR-802 [EARS-U]: A volume write goes through the engine
> WHEN any tool in these families writes into a volume THE tool SHALL write through `core::fs_ops`, subject to the same ACL, write quota and audit rules as `fs.write`.

- **Inputs:** the bytes and the destination path.
- **Outputs:** the committed file plus session accounting (`tools/web.rs:13-15`, `tools/sqlite.rs:5-6`).
- **Business Rules:** this is S2 FR-201 binding on this document. An integration is a way to produce content, never a way around the rules that govern it. The bearer token must be a member of the project and the session write quota applies.
- **Priority:** Must-have

#### FR-803 [EARS-U]: Volume-touching tools take mount_id and pass the gate
> Every tool that reads or writes a volume SHALL take `mount_id` and SHALL authorize through the membership gate before any storage access.

- **Inputs:** `mount_id`, the caller identity.
- **Outputs:** the operation, or `ERR_FORBIDDEN`.
- **Business Rules:** `sqlite.*` and `db.*` always take `mount_id`; `web.fetch` takes it optionally and `web.download` requires it; `context7.*` takes none, because it touches no volume.
- **Priority:** Must-have

### Web

#### FR-804 [EARS-E]: Keyless web search
> WHEN `web.search`, `web.news` or `web.suggestions` is called THE server SHALL query DuckDuckGo, a keyless service, and return the results without requiring an API key.

- **Inputs:** the query; `max_results`.
- **Outputs:** results carrying title, URL and snippet for search and news, and completion strings for suggestions (`tools/web.rs`).
- **Business Rules:** `max_results` is capped at 50 regardless of the request. `web.safe_search` and `web.request_timeout_secs` come from config (`config.rs:486-491`). No API key is needed, which is why this family can be keyless and still useful.
- **Priority:** Must-have

#### FR-805 [EARS-O]: Fetch can bypass the context window
> IF `web.fetch` receives both `mount_id` and `save_path` THEN the server SHALL write the fetched content directly into that volume instead of returning it.

- **Inputs:** the URL, optionally `mount_id` and `save_path`.
- **Outputs:** the content, or the written file (`tools/web.rs:6-8,64-77`).
- **Business Rules:** this context bypass exists for large pages and for cases where the raw HTML is needed, both of which are wasteful or lossy through an LLM context. `save_path` is an absolute POSIX path within the volume.
- **Priority:** Must-have

#### FR-806 [EARS-O]: The two save parameters are all-or-nothing
> IF exactly one of `mount_id` and `save_path` is provided THEN the server SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `mount_id and save_path must both be provided, or neither`.

- **Inputs:** the two optional parameters.
- **Outputs:** the refusal (`tools/web.rs:84-87`).
- **Business Rules:** a half-specified destination is a caller mistake with two plausible readings, so it is refused rather than guessed.
- **Priority:** Must-have

#### FR-807 [EARS-U]: Download always writes to a volume
> `web.download` SHALL always write its content into a volume and SHALL NOT return the bytes to the caller.

- **Inputs:** the URL, `mount_id`, the destination path.
- **Outputs:** the written file (`tools/web.rs:9-11`).
- **Business Rules:** binary data such as images, PDFs and archives cannot pass through an LLM context, so returning it is not an option. This is why `mount_id` is required here and optional on `web.fetch`.
- **Priority:** Must-have

### Documentation

#### FR-808 [EARS-E]: Library identity resolution
> WHEN `context7.resolve_library_id` is called THE server SHALL resolve a library name to its Context7 identifier.

- **Inputs:** `library_name`, for example `react`, `tokio` or `numpy`.
- **Outputs:** the identifier (`tools/context7.rs:13-18`).
- **Business Rules:** the call is documented as the step to take before fetching docs. The API is public and needs no key (`tools/context7.rs:3-4`).
- **Priority:** Must-have

#### FR-809 [EARS-E]: Documentation retrieval
> WHEN `context7.get_library_docs` is called THE server SHALL fetch that library's documentation from Context7 and return it as Markdown.

- **Inputs:** the library identifier.
- **Outputs:** Markdown documentation.
- **Business Rules:** the endpoint base is `https://context7.com/api` by default with a 30 second timeout (`config.rs:508-514`). Markdown is returned rather than HTML because the consumer is an LLM.
- **Priority:** Must-have

### User databases

#### FR-810 [EARS-E]: Extract, operate, commit
> WHEN any `sqlite.*` tool runs THE server SHALL read the database bytes from the volume, write them to a temporary file, open the connection on that file, run the operation, and for a write read the modified bytes back and commit them through the engine.

- **Inputs:** `mount_id`, the `.db` path, the operation.
- **Outputs:** the result, and for writes the committed file (`tools/sqlite.rs:3-11`).
- **Business Rules:** the temporary file exists because rusqlite needs a real POSIX path and volume content is not on the filesystem. The temporary file is deleted automatically when dropped. The commit path is `fs_ops::write_bytes`, so quota, ACL and audit are enforced (FR-802).
- **Priority:** Must-have

#### FR-811 [EARS-U]: One writer per database
> Concurrent access to the same `(mount_id, db_path)` SHALL be serialized.

- **Inputs:** concurrent requests.
- **Outputs:** serialized execution (`tools/sqlite.rs:13,30-37`).
- **Business Rules:** a process-wide lock map keyed by the pair means two requests cannot corrupt the same database. The lock is held for the whole extract-operate-commit cycle, not just the statement (`tools/sqlite.rs:201-202`). This is a per-process lock, so it does not protect against two replicas.
- **Priority:** Must-have

#### FR-812 [EARS-O]: Query is read-only by statement inspection
> IF the statement passed to `sqlite.query` does not begin with `SELECT` or `WITH` THEN the server SHALL refuse it with the message `sqlite.query only accepts SELECT or WITH statements; use sqlite.execute for writes`.

- **Inputs:** the SQL text.
- **Outputs:** the refusal (`tools/sqlite.rs:82-90`).
- **Business Rules:** the check is on the first keyword. It separates the read tool from the write tool so an agent's intent is explicit, and it is a usability guard rather than a security boundary: `sqlite.execute` exists and is equally available to the same caller.
- **Priority:** Must-have

#### FR-813 [EARS-E]: Schema introspection
> WHEN `sqlite.list_tables`, `sqlite.describe_table` or `sqlite.list_indexes` is called THE server SHALL report the database's structure.

- **Inputs:** the `.db` path; optionally a table name to filter indexes.
- **Outputs:** tables and views, column and index descriptions (`tools/sqlite.rs:271-300`).
- **Business Rules:** `list_tables` reads `sqlite_master` for types `table` and `view`, ordered by name. Omitting the table filter on `list_indexes` returns every index.
- **Priority:** Must-have

#### FR-814 [EARS-E]: CSV import, export and maintenance
> WHEN `sqlite.import_csv`, `sqlite.export_csv` or `sqlite.vacuum` is called THE server SHALL perform the operation and commit any resulting change through the engine.

- **Inputs:** the `.db` path plus the CSV path or table name.
- **Outputs:** the modified database or the exported file.
- **Business Rules:** all three follow the extract-operate-commit pattern, so all three charge the quota for the bytes they write back.
- **Priority:** Must-have

#### FR-815 [EARS-U]: Result and time caps
> `sqlite.query` SHALL return at most `sqlite.max_result_rows` rows and SHALL abort a statement exceeding `sqlite.statement_timeout_secs`.

- **Inputs:** the config values.
- **Outputs:** the capped result (`config.rs:531-537`).
- **Business Rules:** `max_result_rows` defaults to 1000 with a hard ceiling of 10000; `statement_timeout_secs` defaults to 30. The caps bound both server work and the tokens returned to an agent.
- **Priority:** Must-have

### Analytics

#### FR-816 [EARS-U]: Table name derivation from the file stem
> The table name in a `db.query` statement SHALL be the file's stem, lowercased.

- **Inputs:** the file path.
- **Outputs:** the registered table name (`tools/db.rs:7-8,55-56`).
- **Business Rules:** `/data/Sales_2024.csv` is queried as `FROM sales_2024`. The rule is stated in the tool's own description, because an agent has no other way to know what to write.
- **Priority:** Must-have

#### FR-817 [EARS-E]: Analytics over stored data files
> WHEN a `db.*` tool runs THE server SHALL read the file's bytes from the volume, write them to a temporary file, register it with DataFusion, and run the operation.

- **Inputs:** `mount_id`, the file path, the operation.
- **Outputs:** rows, a schema, a profile, or a converted file (`tools/db.rs:3-6`).
- **Business Rules:** supported formats are CSV, Parquet and JSON. `db.convert` writes its output back through `fs_ops::write_bytes` (FR-802).
- **Priority:** Must-have

#### FR-818 [EARS-O]: File size limit
> IF a data file is larger than `db.max_file_bytes` THEN the server SHALL refuse the operation.

- **Inputs:** the file's size from `stat`.
- **Outputs:** the refusal (`tools/db.rs:233,258`).
- **Business Rules:** the default is 100 MiB. The size is checked before the bytes are pulled, so an oversized file costs a stat rather than a read.
- **Priority:** Must-have

#### FR-819 [EARS-U]: Row caps
> `db.query` and `db.sample` SHALL return at most `db.max_result_rows` rows.

- **Inputs:** the requested `max_rows`, default 100 for query and capped at 1000 for sample.
- **Outputs:** the capped result (`tools/db.rs:101,249-253`).
- **Business Rules:** `max_result_rows` defaults to 1000 with a hard ceiling of 10000 (`config.rs:547-554`). A caller's request is capped rather than refused.
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
- Every family caps its output: 50 web results, 1000 rows by default with a 10000 ceiling for both database families (FR-804, FR-815, FR-819).
- `db.max_file_bytes` bounds the bytes any single analytics call materializes (FR-818).
- The extract-operate-commit pattern copies a whole database file per call, so cost grows with file size rather than with result size. For a large database this is the dominant cost and there is no incremental path. Recorded as TBD-801.
- External calls carry timeouts: `web.request_timeout_secs` and `context7.request_timeout_secs`, both defaulting to 30 for Context7.

### 7.2 Security
- Every volume write passes the ACL, quota and audit rules (FR-802).
- The per-database lock prevents two requests corrupting one file (FR-811), within one process.
- The `SELECT`-only gate on `sqlite.query` is a usability guard, not a security boundary, and this document says so rather than implying protection it does not give (FR-812).
- Enabling a family is an operator decision about what the server reaches; all four are off by default (FR-801).
- These tools issue outbound requests to public services, which is a deployment consideration for a network-restricted environment.

### 7.3 Usability
- `web.download` writing to a volume rather than returning bytes matches what an LLM can actually consume (FR-807).
- The half-specified destination is refused with a message naming both parameters (FR-806).
- The table-naming rule is stated in the tool description, because an agent cannot infer it (FR-816).
- `sqlite.query` refusing a write names the tool to use instead (FR-812).

### 7.4 Reliability
- Temporary files are deleted on drop (FR-810).
- The lock is held across the whole cycle, so a crash between read and commit leaves the stored file unchanged rather than half-written (FR-811).
- Capped results keep one call from exhausting memory.

### 7.5 Observability
Unchanged from S1 §7.5. Writes produce audit entries like any other write, so a file created by `web.download` or `db.convert` is traceable through `fs.audit_log`. Outbound request duration and failure rate are not measured, so a degraded external service is visible only as slow tool calls. Recorded as TBD-802.

### 7.6 Deployment
- All four off by default; enable per family in YAML or with `--web`, `--context7`, `--sqlite`, `--db`.
- `web` needs outbound access to DuckDuckGo; `context7` to `https://context7.com/api`.
- Neither needs an API key.
- `sqlite` and `db` need only temporary-directory space proportional to the files they touch.

### 7.7 Scalability
- The per-database lock is per process, so two replicas can write the same stored database concurrently and the last commit wins. Recorded as TBD-803.
- Temporary file space scales with concurrent calls times file size.
- Outbound rate limits on the public services are not managed by the server; a burst of `web.search` calls is passed straight through.

## 8. Data Model

No persisted server entity. The artifacts are:

| Artifact | Shape | Home |
|---|---|---|
| Web result | title, URL, snippet | `tools/web.rs` |
| Saved page or download | a file in the volume | S1's Volume |
| Library docs | Markdown | `tools/context7.rs` |
| User database | a `.db` file in the volume, owned by the caller | S1's Volume |
| Query result | `{columns, rows, row_count}` | `tools/sqlite.rs:94` |
| Data file | CSV, Parquet or JSON in the volume | S1's Volume |

## 9. Impact Analysis

### 9.1 Affected Components

| File/Module | Impact Type | Description |
|---|---|---|
| `crates/mcp-fs/src/tools/web.rs` | Specified, unchanged | FR-804..FR-807 |
| `crates/mcp-fs/src/tools/context7.rs` | Specified, unchanged | FR-808, FR-809 |
| `crates/mcp-fs/src/tools/sqlite.rs` | Specified, unchanged | FR-810..FR-815 |
| `crates/mcp-fs/src/tools/db.rs` | Specified, unchanged | FR-816..FR-819 |
| `crates/mcp-fs/src/tools/all.rs` | Referenced | FR-801 |
| `crates/mcp-fs/src/config.rs` | Referenced | the four config blocks |

### 9.2 Affected Requirements

S1 FR-004 (CLI flags force a family on) and S2 FR-201 (the engine is the only implementation) bind here as FR-801 and FR-802. Nothing is modified.

### 9.3 Affected Tests

| Test location | Coverage today | Action |
|---|---|---|
| `crates/mcp-fs/src/tools/sqlite.rs` `#[cfg(test)]` | statement gate, locking, round-trip | Keep; annotate |
| `crates/mcp-fs/src/tools/db.rs` `#[cfg(test)]` | table naming, caps, formats | Keep; annotate |
| `crates/mcp-fs/src/tools/web.rs` `#[cfg(test)]` | parameter validation | Keep; annotate |
| `crates/mcp-fs/src/tools/all.rs:82-100` | registration counts with families enabled | Keep; annotate |
| `tests/functional/scenarios/08_database.sh` | database tools over HTTP | Extend |

The `web.*` and `context7.*` families call public services, so their happy paths are **not** testable without network access. §12 specifies them against stub endpoints and marks the live-service cases as out of scope for CI.

### 9.4 Affected Documentation

| Document | Section | Action |
|---|---|---|
| `AGENTS.md` | documentation index | Reference this spec |
| `.agent_docs/tools.md` | optional families | Cross-reference §6 |

### 9.5 Dependencies & Risks

1. **Two public services with no contract.** DuckDuckGo and Context7 can change shape or block traffic at any time, and neither offers a stability guarantee. Tests must not depend on them.
2. **Extract-operate-commit does not scale to large databases** (TBD-801).
3. **The per-database lock is per process** (TBD-803).
4. **`sqlite.execute` runs arbitrary SQL** against a caller's own database. That is the tool's purpose, and the blast radius is one file the caller already controls, but it is worth naming explicitly.

## 10. Documentation Requirements

### 10.1 README.md
List the four optional families, their flags, and the fact that none needs an API key.

### 10.2 AGENTS.md & .agent_docs/
- `AGENTS.md`: add this spec to the index.
- `.agent_docs/tools.md`: cross-reference §6 for these families.

### 10.3 docs/*
None required.

## 11. Traceability Matrix

| Scenario | Functional Req | E2E Tests (Happy) | E2E Tests (Failure) | E2E Tests (Edge) |
|---|---|---|---|---|
| SC-801 | FR-804, FR-805, FR-806, FR-807 | E2E-801 | E2E-802, E2E-803, E2E-804 | E2E-805, E2E-806 |
| SC-802 | FR-808, FR-809 | E2E-807 | E2E-808, E2E-809 | E2E-810 |
| SC-803 | FR-810, FR-811, FR-812, FR-813, FR-814, FR-815 | E2E-811, E2E-812 | E2E-813, E2E-814, E2E-815, E2E-816 | E2E-817, E2E-818, E2E-819 |
| SC-804 | FR-816, FR-817, FR-818, FR-819 | E2E-820, E2E-821 | E2E-822, E2E-823, E2E-824 | E2E-825, E2E-826 |
| SC-805 | FR-801, FR-802, FR-803 | E2E-827 | E2E-828, E2E-829, E2E-830 | E2E-831, E2E-832 |

Per-FR coverage:

| FR | Tests | FR | Tests |
|---|---|---|---|
| FR-801 | E2E-827, E2E-828, E2E-831 | FR-811 | E2E-812, E2E-814, E2E-818 |
| FR-802 | E2E-802, E2E-829, E2E-832 | FR-812 | E2E-813, E2E-817, E2E-819 |
| FR-803 | E2E-803, E2E-829, E2E-830 | FR-813 | E2E-811, E2E-815, E2E-817 |
| FR-804 | E2E-801, E2E-804, E2E-805 | FR-814 | E2E-812, E2E-816, E2E-818 |
| FR-805 | E2E-801, E2E-802, E2E-806 | FR-815 | E2E-815, E2E-816, E2E-819 |
| FR-806 | E2E-803, E2E-805, E2E-806 | FR-816 | E2E-820, E2E-822, E2E-825 |
| FR-807 | E2E-802, E2E-804, E2E-806 | FR-817 | E2E-820, E2E-821, E2E-823 |
| FR-808 | E2E-807, E2E-808, E2E-810 | FR-818 | E2E-822, E2E-824, E2E-826 |
| FR-809 | E2E-807, E2E-809, E2E-810 | FR-819 | E2E-821, E2E-823, E2E-825 |
| FR-810 | E2E-811, E2E-813, E2E-817 | | |

## 12. End-to-End Test Suite

**Placement.** Tool-level tests are Rust tests in the `#[cfg(test)]` modules of the four tool files, plus `tools/all.rs` for registration counts. Database tests extend `tests/functional/scenarios/08_database.sh`.

**Network policy.** No test in this suite contacts DuckDuckGo or Context7. Web and documentation tests run against a local stub HTTP server, and the live services are exercised only by manual smoke testing.

**Fixtures:** project `spec-int` with `ALICE` as member and `BOB` as a non-member; `/data/Sales_2024.csv` with a header and three rows; `/data/big.csv` of 150 MiB; `/db/app.db` with one table `users` holding two rows; a stub HTTP server returning fixed search results and Markdown docs.

### 12.1 Test Summary

| Test ID | Action | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-801 | New | Core Journey | SC-801 | FR-804, FR-805 | Critical |
| E2E-802 | New | Side Effect | SC-801 | FR-802, FR-805, FR-807 | Critical |
| E2E-803 | Existing | Error | SC-801 | FR-803, FR-806 | Critical |
| E2E-804 | New | Error | SC-801 | FR-804, FR-807 | High |
| E2E-805 | New | Edge | SC-801 | FR-804, FR-806 | High |
| E2E-806 | New | Edge | SC-801 | FR-805, FR-806, FR-807 | High |
| E2E-807 | New | Core Journey | SC-802 | FR-808, FR-809 | High |
| E2E-808 | New | Error | SC-802 | FR-808 | High |
| E2E-809 | New | Error | SC-802 | FR-809 | High |
| E2E-810 | New | Edge | SC-802 | FR-808, FR-809 | Medium |
| E2E-811 | Existing | Core Journey | SC-803 | FR-810, FR-813 | Critical |
| E2E-812 | Existing | Feature | SC-803 | FR-811, FR-814 | Critical |
| E2E-813 | Existing | Error | SC-803 | FR-810, FR-812 | Critical |
| E2E-814 | New | Data Integrity | SC-803 | FR-811 | Critical |
| E2E-815 | New | Error | SC-803 | FR-813, FR-815 | High |
| E2E-816 | New | Error | SC-803 | FR-814, FR-815 | High |
| E2E-817 | New | Edge | SC-803 | FR-810, FR-812, FR-813 | Critical |
| E2E-818 | New | Edge | SC-803 | FR-811, FR-814 | High |
| E2E-819 | New | Edge | SC-803 | FR-812, FR-815 | High |
| E2E-820 | Existing | Core Journey | SC-804 | FR-816, FR-817 | Critical |
| E2E-821 | Existing | Feature | SC-804 | FR-817, FR-819 | Critical |
| E2E-822 | New | Error | SC-804 | FR-816, FR-818 | High |
| E2E-823 | New | Error | SC-804 | FR-817, FR-819 | High |
| E2E-824 | New | Error | SC-804 | FR-818 | Critical |
| E2E-825 | New | Edge | SC-804 | FR-816, FR-819 | High |
| E2E-826 | New | Edge | SC-804 | FR-818 | Medium |
| E2E-827 | Existing | Core Journey | SC-805 | FR-801 | Critical |
| E2E-828 | New | Error | SC-805 | FR-801 | Critical |
| E2E-829 | New | Security | SC-805 | FR-802, FR-803 | Critical |
| E2E-830 | New | Security | SC-805 | FR-803 | Critical |
| E2E-831 | New | Edge | SC-805 | FR-801 | High |
| E2E-832 | New | Edge | SC-805 | FR-802 | High |

**Coverage Statistics** (32 tests):
- Happy path (Core Journey + Feature): 8
- Failure/error (Error + Security): 16
- Side effects: 1
- Edge cases: 6
- Data integrity: 1
- Happy:Failure ratio: 1:2.0

### 12.2 New Test Specifications

#### E2E-801: Search returns titled results against a stub
- **Category:** Core Journey | **Scenario:** SC-801 | **Requirements:** FR-804, FR-805
- **Preconditions:** `web.enabled` true, pointed at a stub returning two fixed results.
- **Steps:**
  - Given the stub
  - When `web.search` is called with the query `rust async`
  - Then two results are returned, each carrying a title, a URL and a snippet
  - And `web.fetch` on the first result's URL returns its body content
- **Priority:** Critical

#### E2E-802: A saved fetch is charged and audited like any write
- **Category:** Side Effect | **Scenario:** SC-801 | **Requirements:** FR-802, FR-805, FR-807
- **Preconditions:** a fresh session for `ALICE`; the stub serving a 500-byte page.
- **Steps:**
  - Given a fresh session
  - When `web.fetch` is called with `mount_id` `spec-int` and `save_path` `/page.html`
  - Then the file exists with the fetched bytes
  - And `fs.audit_log` contains one entry for `/page.html`
  - And a `web.download` of a binary fixture to `/img.png` also produces an audit entry
  - And a second save that exceeds `safety.write_quota_bytes` fails with `ERR_WRITE_QUOTA_EXCEEDED` and writes nothing
- **Cleanup:** delete both files
- **Priority:** Critical

#### E2E-804: Download requires a destination
- **Category:** Error | **Scenario:** SC-801 | **Requirements:** FR-804, FR-807
- **Steps:**
  - Given `web.download` called with a URL and no `mount_id`
  - Then the call fails with `ERR_INVALID_ARGUMENT`
  - And the response never contains the binary payload
  - And with a valid `mount_id` and destination it succeeds and returns a path rather than bytes
- **Priority:** High

#### E2E-805: Result count is capped at 50
- **Category:** Edge | **Scenario:** SC-801 | **Requirements:** FR-804, FR-806
- **Preconditions:** a stub able to return 100 results.
- **Steps:**
  - Given `max_results` requested as 100
  - When `web.search` is called
  - Then at most 50 results are returned
  - And the call succeeds rather than failing on the oversized request
- **Priority:** High

#### E2E-806: The save parameters are all-or-nothing in both directions
- **Category:** Edge | **Scenario:** SC-801 | **Requirements:** FR-805, FR-806, FR-807
- **Steps:**
  - Given `web.fetch` called with `mount_id` and no `save_path`
  - Then it fails with `ERR_INVALID_ARGUMENT` and the message `mount_id and save_path must both be provided, or neither`
  - And the same failure occurs with `save_path` and no `mount_id`
  - And with neither, the content is returned to the caller
  - And with both, the content is written and not returned
- **Priority:** High

#### E2E-807: Library resolution then documentation
- **Category:** Core Journey | **Scenario:** SC-802 | **Requirements:** FR-808, FR-809
- **Preconditions:** `context7.enabled` true, `api_url` pointed at a stub.
- **Steps:**
  - Given the stub resolving `tokio` to a fixed identifier
  - When `context7.resolve_library_id` is called with `tokio`
  - Then the identifier is returned
  - And `context7.get_library_docs` with that identifier returns Markdown text
- **Priority:** High

#### E2E-808: An unknown library is not a server error
- **Category:** Error | **Scenario:** SC-802 | **Requirements:** FR-808
- **Steps:**
  - Given the stub returning no match for `definitely-not-a-library`
  - When `context7.resolve_library_id` is called
  - Then the call succeeds with an empty or unresolved result rather than raising `ERR_INTERNAL_ERROR`
- **Priority:** High

#### E2E-809: An unreachable API times out cleanly
- **Category:** Error | **Scenario:** SC-802 | **Requirements:** FR-809
- **Preconditions:** `api_url` pointed at a port that accepts and never answers; `request_timeout_secs` set to 2.
- **Steps:**
  - Given that configuration
  - When `context7.get_library_docs` is called
  - Then the call fails within 5 seconds
  - And the server remains responsive, confirmed by a subsequent `/health` request
- **Priority:** High

#### E2E-810: The documentation tools touch no volume
- **Category:** Edge | **Scenario:** SC-802 | **Requirements:** FR-808, FR-809
- **Steps:**
  - Given `tools/list`
  - When the two `context7.*` schemas are inspected
  - Then neither declares a `mount_id` parameter
  - And calling them with no project membership anywhere still succeeds
- **Priority:** Medium

#### E2E-814: Concurrent writes to one database are serialized
- **Category:** Data Integrity | **Scenario:** SC-803 | **Requirements:** FR-811
- **Preconditions:** `/db/app.db` with a table `counter` holding a single row at 0.
- **Steps:**
  - Given ten concurrent `sqlite.execute` calls each incrementing that row by 1
  - When all ten complete
  - Then `sqlite.query` reports the value 10, with no lost update
  - And the database file is not corrupted, confirmed by `sqlite.list_tables` succeeding afterwards
- **Priority:** Critical

#### E2E-815: Row results are capped
- **Category:** Error | **Scenario:** SC-803 | **Requirements:** FR-813, FR-815
- **Preconditions:** a database whose table holds 5000 rows; `sqlite.max_result_rows` set to 100.
- **Steps:**
  - Given that configuration
  - When `sqlite.query` selects every row
  - Then at most 100 rows are returned
  - And `row_count` reflects what was returned
  - And the call succeeds rather than failing
- **Priority:** High

#### E2E-816: Import and export round-trip through the engine
- **Category:** Error | **Scenario:** SC-803 | **Requirements:** FR-814, FR-815
- **Preconditions:** `/data/Sales_2024.csv` with three rows; a fresh session.
- **Steps:**
  - Given `sqlite.import_csv` importing that file into `/db/app.db` as table `sales`
  - Then `sqlite.query` on `sales` returns three rows
  - And `fs.audit_log` records the write to `/db/app.db`
  - And `sqlite.export_csv` writes the table back out to a new path, which also appears in the audit log
- **Cleanup:** delete the exported file
- **Priority:** High

#### E2E-817: The read tool refuses writes and the write tool accepts them
- **Category:** Edge | **Scenario:** SC-803 | **Requirements:** FR-810, FR-812, FR-813
- **Preconditions:** `/db/app.db` with table `users`.
- **Steps:**
  - Given `sqlite.query` called with `DELETE FROM users`
  - Then it fails with the message `sqlite.query only accepts SELECT or WITH statements; use sqlite.execute for writes`
  - And `users` still holds its two rows
  - And the same statement through `sqlite.execute` succeeds
  - And a `WITH ... SELECT` statement is accepted by `sqlite.query`
- **Cleanup:** restore the fixture
- **Priority:** Critical

#### E2E-818: A read leaves the stored file byte-identical
- **Category:** Edge | **Scenario:** SC-803 | **Requirements:** FR-811, FR-814
- **Preconditions:** `/db/app.db`; its `fs.hash` recorded as H.
- **Steps:**
  - Given the recorded hash
  - When `sqlite.query`, `sqlite.list_tables`, `sqlite.describe_table` and `sqlite.list_indexes` are each called
  - Then `fs.hash` still equals H, proving a read commits nothing
  - And `fs.audit_log` contains no entry for the database
  - And after `sqlite.vacuum` the hash differs and an audit entry exists
- **Priority:** High

#### E2E-819: Statement timeout aborts a long query
- **Category:** Edge | **Scenario:** SC-803 | **Requirements:** FR-812, FR-815
- **Preconditions:** `statement_timeout_secs` set to 1.
- **Steps:**
  - Given a recursive CTE that runs far longer than one second
  - When `sqlite.query` is called with it
  - Then the call fails rather than hanging
  - And it returns within a few seconds
  - And a subsequent quick query against the same database succeeds, proving the lock was released
- **Priority:** High

#### E2E-822: An unknown table name is a clear error
- **Category:** Error | **Scenario:** SC-804 | **Requirements:** FR-816, FR-818
- **Preconditions:** `/data/Sales_2024.csv`.
- **Steps:**
  - Given `db.query` with `SELECT * FROM sales`
  - Then it fails with an error naming the unknown table
  - And with `FROM sales_2024`, the file stem lowercased, it succeeds
  - And the returned columns match the CSV header
- **Priority:** High

#### E2E-823: Row caps apply to query and sample
- **Category:** Error | **Scenario:** SC-804 | **Requirements:** FR-817, FR-819
- **Preconditions:** a CSV with 5000 rows; `db.max_result_rows` set to 50.
- **Steps:**
  - Given that configuration
  - When `db.query` is called with `max_rows` 4000
  - Then at most 50 rows are returned
  - And `db.sample` with `rows` 4000 likewise returns at most 50
  - And both calls succeed
- **Priority:** High

#### E2E-824: An oversized file is refused before it is read
- **Category:** Error | **Scenario:** SC-804 | **Requirements:** FR-818
- **Preconditions:** `/data/big.csv` of 150 MiB; `db.max_file_bytes` at its 100 MiB default.
- **Steps:**
  - Given that file
  - When `db.schema` is called on it
  - Then the call fails naming the size limit
  - And the failure is fast, consistent with a stat rather than a full read
  - And the same call on a 1 MiB file succeeds
- **Priority:** Critical

#### E2E-825: Each supported format is readable and convertible
- **Category:** Edge | **Scenario:** SC-804 | **Requirements:** FR-816, FR-819
- **Preconditions:** the same data as CSV, Parquet and JSON.
- **Steps:**
  - Given all three files
  - When `db.schema` is called on each
  - Then all three report the same column names and types
  - And `db.convert` from CSV to Parquet produces a file that `db.schema` reads with the same columns
  - And the converted file appears in `fs.audit_log`
- **Cleanup:** delete the converted files
- **Priority:** High

#### E2E-826: The size check precedes the format check
- **Category:** Edge | **Scenario:** SC-804 | **Requirements:** FR-818
- **Preconditions:** a 150 MiB file with an unsupported extension.
- **Steps:**
  - Given that file
  - When `db.schema` is called
  - Then the call fails
  - And the error names the size limit rather than the unsupported format, documenting the check order
- **Priority:** Medium

#### E2E-828: A disabled family's tools are absent, not failing
- **Category:** Error | **Scenario:** SC-805 | **Requirements:** FR-801
- **Preconditions:** a server with all four families disabled.
- **Steps:**
  - Given that server
  - When `tools/list` is called
  - Then no name begins with `web.`, `context7.`, `sqlite.` or `db.`
  - And the total count is 45
  - And `tools/call` for `web.search` returns the JSON-RPC error `-32602` with message `Unknown tool: 'web.search'`
- **Cleanup:** stop the server
- **Priority:** Critical

#### E2E-829: A non-member cannot write through an integration
- **Category:** Security | **Scenario:** SC-805 | **Requirements:** FR-802, FR-803
- **Preconditions:** `BOB` authenticated and not a member of `spec-int`.
- **Steps:**
  - Given `BOB`'s bearer
  - When `web.download` targets `spec-int`
  - Then it fails with `ERR_FORBIDDEN`
  - And `sqlite.query` against `/db/app.db` in `spec-int` fails the same way
  - And `db.schema` on a file in `spec-int` fails the same way
  - And nothing was written or read in any case
- **Priority:** Critical

#### E2E-830: A platform admin gains no access through an integration
- **Category:** Security | **Scenario:** SC-805 | **Requirements:** FR-803
- **Preconditions:** `ADMIN` is a platform admin and not a member of `spec-int`.
- **Steps:**
  - Given `ADMIN`'s bearer
  - When `sqlite.list_tables` is called against `spec-int`
  - Then it fails with `ERR_FORBIDDEN`
  - And `db.sample` fails the same way
  - And `ADMIN` can still call `admin.list_all_projects` successfully
- **Priority:** Critical

#### E2E-831: Enabling one family does not enable the others
- **Category:** Edge | **Scenario:** SC-805 | **Requirements:** FR-801
- **Preconditions:** a server started with `--sqlite` only.
- **Steps:**
  - Given that server
  - When `tools/list` is called
  - Then exactly 8 names begin with `sqlite.`
  - And no name begins with `web.`, `context7.` or `db.`
  - And the total count is 53
- **Cleanup:** stop the server
- **Priority:** High

#### E2E-832: Every integration write lands in the audit log
- **Category:** Edge | **Scenario:** SC-805 | **Requirements:** FR-802
- **Preconditions:** all four families enabled; a fresh session; stub endpoints for the two network families.
- **Steps:**
  - Given a fresh session
  - When a `web.download`, a `sqlite.execute`, a `sqlite.export_csv` and a `db.convert` are each performed
  - Then `fs.audit_log` contains one entry per resulting file
  - And each entry's `detail` reports a byte count
  - And the session's quota has advanced by the total of those byte counts
- **Cleanup:** delete the created files
- **Priority:** High

### 12.3 Modified Test Specifications

None.

### 12.4 Removed Tests

None.

## 13. Consistency Notes

1. **Two senses of "database".** S4 governs the server's own relational state; S8's `sqlite.*` family operates on user databases stored as volume files. They share no code path: `sqlite.*` uses rusqlite on a temporary file, not `RelationalDb`. §4.5 names the distinction.
2. **The `SELECT`-only gate is not a security boundary** (FR-812). `sqlite.execute` is available to the same caller, so the gate expresses intent rather than restricting authority. Stating this prevents a reader from treating `sqlite.query` as a read-only capability.
3. **S1's registry tests pin the counts** this document asserts: 5 web, 2 context7, 8 sqlite, 5 db. E2E-831's total of 53 follows from 45 + 8.

## 14. Migration & Implementation Notes

No production code change. Test work, in order:

1. **Annotate existing tests** in the four tool modules and `tools/all.rs`.
2. **Add the registration tests** first (E2E-828, E2E-831): they need only a server and `tools/list`, and they pin the counts every other test's fixtures assume.
3. **Add the authorization tests** (E2E-829, E2E-830), which reuse the `BOB` and `ADMIN` fixtures from S1.
4. **Add the sqlite and db tests**, which need no network: E2E-814 through E2E-819 and E2E-822 through E2E-826. E2E-814 is the valuable one, because the lock is the family's only concurrency control.
5. **Build the stub HTTP server before the web and context7 tests.** No test may contact DuckDuckGo or Context7; the stub serves fixed search results, a fixed library resolution and fixed Markdown. E2E-809 needs a port that accepts and never answers, which the stub must provide deliberately.
6. **E2E-824 needs a 150 MiB fixture.** Generate it in the test rather than committing it, and delete it afterwards.
7. **E2E-815, E2E-819, E2E-823 and E2E-824 need non-default configuration**, so each starts its own server rather than mutating a shared fixture.

## 15. Open Questions & TBDs

- **TBD-801:** Extract-operate-commit copies the whole database or data file per call (§7.1). For a large SQLite database every query pays a full read and every write a full read plus a full write. There is no incremental path, and whether one is wanted is undecided.
- **TBD-802:** Outbound request duration and failure rate are unmeasured (§7.5). A degraded public service appears only as slow tool calls.
- **TBD-803:** The per-database lock is per process (§7.7). Two replicas can write the same stored database concurrently and the last commit wins, silently losing the other's changes.
- **TBD-804:** Neither network family rate-limits its outbound calls. A burst of agent searches is passed straight through to a public service that may throttle or block the deployment.

## 16. Glossary

| Term | Definition | Context |
|---|---|---|
| **Optional family** | A tool family registered only when its config flag is set. | Volume |
| **Keyless service** | An external API reachable without credentials, as DuckDuckGo and Context7 are. | Web |
| **Context bypass** | Writing fetched content straight into a volume instead of returning it to the LLM. | Web |
| **Library id** | The Context7 identifier a library name resolves to before its docs are fetched. | Documentation |
| **User database** | A SQLite database stored as a file in a volume, owned by the caller. | User database |
| **Extract-operate-commit** | Materializing a stored file to a temporary path, operating on it, and committing the result through the engine. | User database |
| **Per-database lock** | The process-wide lock keyed by mount and path that serializes access to one stored database. | User database |
| **Statement gate** | The first-keyword check restricting `sqlite.query` to `SELECT` and `WITH`. | User database |
| **Table name derivation** | The rule making a data file's stem, lowercased, its SQL table name. | Analytics |
| **Row cap** | The configured ceiling on rows returned by a database or analytics query. | Analytics |
| **File size limit** | The configured ceiling on the size of a data file an analytics call will read. | Analytics |

## 17. Interview Decisions Log

Produced non-interactively from the code.

- **DEC-801:** The four families are specified in one document rather than four. **Rationale:** they share one shape, off by default, gated by a flag, writing back only through the engine, and three of the four are small; four documents would repeat that shape four times and obscure it. **Alternatives considered:** one spec per family. **Implemented by:** FR-801, FR-802, FR-803. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/tools/all.rs:37-43`.
- **DEC-802:** The `SELECT`-only gate is specified as a usability guard and explicitly not as a security boundary. **Rationale:** `sqlite.execute` is available to the same caller, so treating the gate as protection would be a false claim in a document people will rely on. **Alternatives considered:** presenting it as a read-only capability. **Implemented by:** FR-812, §7.2, §13 item 2. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/tools/sqlite.rs:82-90` alongside the unrestricted `sqlite.execute` registration.
- **DEC-803:** No test contacts the live public services. **Rationale:** a test depending on DuckDuckGo is a test that fails when DuckDuckGo changes, blocks the CI network, or rate-limits; the resulting red build says nothing about this codebase. **Alternatives considered:** live smoke tests gated on a network flag, which tends to become permanently skipped. **Implemented by:** §12 network policy, §14 step 5. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/tools/web.rs:1-4` and `crates/mcp-fs/src/tools/context7.rs:3-4` both call public endpoints directly.
- **DEC-804:** The per-database lock's process scope is stated as a limitation rather than left implied. **Rationale:** the lock reads as complete concurrency control until you ask what happens with two replicas, and the answer is silent data loss. **Alternatives considered:** describing the lock without the caveat. **Implemented by:** FR-811, TBD-803, E2E-814. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/tools/sqlite.rs:13,30-37`, a process-wide static map.
- **DEC-805:** The table-naming rule is specified as a requirement rather than as documentation. **Rationale:** an agent cannot infer that `/data/Sales_2024.csv` is queried as `sales_2024`, so the rule is part of the callable contract and a change to it would break every stored prompt that relies on it. **Alternatives considered:** leaving it in the tool description only, which is where it also lives. **Implemented by:** FR-816, E2E-822. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/tools/db.rs:7-8,55-56`.

## 18. Implementability Audit

Audited as part of the cross-spec audit of S1 through S8 on 2026-09-18; see
`specs/AUDIT.md` for the method, the full findings and the limitations. Sub-agent
execution was unavailable (provider budget error), so the contract's fresh-context
auditor was replaced by mechanical verification plus a targeted reading pass. That
substitution is weaker in one specific respect, recorded in `AUDIT.md`.

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|-------|--------------------------|-------------------|---------|
| 1 | 0 | 0 | IMPLEMENTABLE-WITH-DRIFT |

**Amendments applied:** none required
**Drift registered:** none. Every A finding was evident and amended in place, which
the contract prefers to a register entry.

## 19. Implementation Drift Register

Empty, and deliberately so: all A findings from the cross-spec audit were evident
corrections applied in place rather than drift to resolve during implementation.
See `specs/AUDIT.md` for each finding and its evidence.
