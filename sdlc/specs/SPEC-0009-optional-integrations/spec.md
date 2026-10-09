> Id: SPEC-0009
> Nature: FEAT
> Status: as-built
> Area: optional-integrations
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# mcp-fs Optional Integrations — Specification

## 1. Summary

Four optional tool families extend an agent's reach beyond the filesystem: **`web.*`** (5 tools, keyless DuckDuckGo search and page retrieval), **`context7.*`** (2 tools, library documentation lookup), **`sqlite.*`** (8 tools, persistent SQLite databases stored as volume files), and **`db.*`** (5 tools, DataFusion analytics over CSV, Parquet and JSON).

Each family is off by default and registered only when its config flag is set. Each reaches something the filesystem does not hold on its own: the public web, a documentation API, or a query engine. Whenever a family writes into a volume it does so through `core::fs_ops`, so the ACL, the write quota and the audit log apply exactly as they do to `fs.write`. An integration is a new way to produce content, never a way around the rules that govern content.

`sqlite.*` and `db.*` share a further pattern: **extract-operate-commit**. The stored file is materialized to a temporary path because the underlying engine (rusqlite / DataFusion) needs a real POSIX path, the operation runs there, and any result is written back through the engine.

## 2. Current State

### 2.1 None (greenfield within this document's scope)

Not applicable: this is a retro-specification of already-shipped behaviour, not a new feature being added to a described legacy state.

### 2.2 None

Not applicable, see 2.1.

### 2.3 As-built state

Four modules under `crates/core/src/tools/`: `web.rs`, `context7.rs`, `sqlite.rs`, `db.rs`, each exposing a `register(reg, config)` entry point called only when its flag is set, from `tools/all.rs`. Together they add 20 tools to the always-on contract.

- **Registration**: `crates/core/src/tools/all.rs` — one `if features.<family>` branch per family, each calling that family's `register`.
- **Config**: `WebConfig`, `Context7Config`, `SqliteConfig`, `DbConfig`, all in `crates/core/src/config.rs`.
- **Locking**: a process-wide lock map keyed by `(mount_id, db_path)` in `crates/core/src/tools/sqlite.rs`, serializing all access to one stored database.
- **Statement gate**: `sqlite.query` accepts only statements beginning `SELECT` or `WITH` (`crates/core/src/tools/sqlite.rs`).
- **Table naming**: `db.*` derives the SQL table name from the data file's stem, lowercased (`crates/core/src/tools/db.rs`).

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

### 3.2 Out of Scope

- Authorization, the error vocabulary, config loading (owned by the membership/core-platform spec).
- The filesystem engine itself (`core::fs_ops`); these families call it, they do not reimplement it.
- The server's own relational state store; `sqlite.*` operates on **user** databases stored as volume files, a different thing.
- Search indexing side effects that fire for these writes as for any other write.
- Any guarantee about an external service's availability, content or stability. DuckDuckGo and Context7 are public services this server calls.

## 4. Actors

| Actor | Description |
|---|---|
| **LLM agent** | The consumer. Searches the web for current information, looks up library documentation before writing code, queries a stored database, analyses a CSV. |
| **Project member** | The identity every volume write is attributed to, subject to the same quota and audit as any write. |
| **Operator** | Decides which families exist. Each is off by default, so enabling one is a deliberate choice about what the server reaches. |
| **External service** | DuckDuckGo and Context7. Both keyless, both public, both outside the deployment's control. |

**Bounded contexts.**

| Context | Scope | Key entities |
|---|---|---|
| **Web** | Public search and page retrieval. | query, result, page, download |
| **Documentation** | Library identity and its docs. | library id, docs |
| **User database** | SQLite databases stored as volume files. | db file, statement, table, index |
| **Analytics** | Columnar query over stored data files. | table name, schema, row, format |
| **Volume** | Where anything written lands. | path, quota, audit entry |

A "database" in the User database context is a file in a volume that a caller owns; it is not the server's relational state. A "table" in the Analytics context is a name derived from a file, not a persisted object.

## 5. Usage Scenarios

### SC-001: Agent searches the web and saves a page

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
- `mount_id` without `save_path`, or the reverse → `ERR_INVALID_ARGUMENT`, `"mount_id and save_path must both be provided, or neither"`
- Writing to a project the caller does not belong to → `ERR_FORBIDDEN`
- The write exceeds the session quota → `ERR_WRITE_QUOTA_EXCEEDED`
- The external service unreachable or slow → the request times out per `request_timeout_secs`
- More results requested than the cap → capped at 50

### SC-002: Agent looks up a library's documentation

**Actor:** LLM agent
**Preconditions:** `context7.enabled` true.
**Flow:**
1. Agent calls `context7.resolve_library_id` with a library name such as `tokio`.
2. Agent calls `context7.get_library_docs` with the resolved id.
3. The documentation comes back as Markdown, which the agent reads directly.

**Postconditions:** the agent holds current documentation rather than relying on its training data.
**Exceptions:**
- An unknown library → an empty or unresolved result, not a server error
- The API unreachable → a timeout after `request_timeout_secs`, default 30
- Calling `get_library_docs` with a name rather than a resolved id → whatever the API returns; resolution is a documented precondition, not an enforced one

### SC-003: Agent queries a SQLite database stored in a volume

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
- A non-`SELECT` statement passed to `sqlite.query` → refused with `"sqlite.query only accepts SELECT or WITH statements; use sqlite.execute for writes"`
- Two concurrent requests against the same database → serialized by the lock, never concurrent
- More rows than `max_result_rows` → capped
- A statement exceeding `statement_timeout_secs` → aborted
- The `.db` path is not in a member's project → `ERR_FORBIDDEN`

### SC-004: Agent analyses a data file

**Actor:** LLM agent
**Preconditions:** `db.enabled` true; the caller is a member; a CSV, Parquet or JSON file exists.
**Flow:**
1. Agent calls `db.schema` to learn the columns, or `db.sample` for rows, or `db.profile` for statistics.
2. Agent calls `db.query` with SQL whose table name is the file stem lowercased: `/data/Sales_2024.csv` is queried as `FROM sales_2024`.
3. The file's bytes are written to a temporary file and registered with DataFusion.
4. `db.convert` writes a converted file back into the volume through the engine.

**Postconditions:** the agent holds the analysis; a conversion leaves a new file with full accounting.
**Exceptions:**
- A file larger than `max_file_bytes`, default 100 MiB → refused
- More rows than `max_result_rows` → capped, hard ceiling 10000
- SQL naming a table that is not the file stem → a DataFusion error naming the unknown table
- An unsupported format → refused

### SC-005: Operator chooses which integrations exist

**Actor:** Operator
**Preconditions:** a running deployment.
**Flow:**
1. Operator leaves every family disabled by default.
2. Operator enables a family in YAML, or forces it on with a CLI flag.
3. The tools appear in the catalogue; the others remain absent.

**Postconditions:** the server reaches exactly what the operator chose.
**Exceptions:**
- A disabled family's tool called → the JSON-RPC unknown-tool error, because the tool is absent rather than present-and-failing
- A CLI flag forces the family on regardless of the YAML value

## 6. Functional Requirements

### Common rules

#### FR-001 [EARS-O]: Each optional family is off by default and gated by its flag
> IF an optional family's `enabled` flag is false THEN none of its tools SHALL be registered.

- **Inputs:** `web.enabled`, `context7.enabled`, `sqlite.enabled`, `db.enabled`, and the matching CLI overrides.
- **Outputs:** the registered tool set.
- **Business Rules:** all four default to false. The counts are 5 web, 2 context7, 8 sqlite, 5 db. An LLM must not see a tool it cannot call, which is why the tools are absent rather than registered and failing.
- **Priority:** Must-have

#### FR-002 [EARS-U]: A volume write goes through the engine
> WHEN any tool in these families writes into a volume THE tool SHALL write through `core::fs_ops`, subject to the same ACL, write quota and audit rules as `fs.write`.

- **Inputs:** the bytes and the destination path.
- **Outputs:** the committed file plus session accounting.
- **Business Rules:** an integration is a way to produce content, never a way around the rules that govern it. The bearer token must be a member of the project and the session write quota applies.
- **Priority:** Must-have

#### FR-003 [EARS-U]: Volume-touching tools take mount_id and pass the gate
> Every tool that reads or writes a volume SHALL take `mount_id` and SHALL authorize through the membership gate before any storage access.

- **Inputs:** `mount_id`, the caller identity.
- **Outputs:** the operation, or `ERR_FORBIDDEN`.
- **Business Rules:** `sqlite.*` and `db.*` always take `mount_id`; `web.fetch` takes it optionally and `web.download` requires it; `context7.*` takes none, because it touches no volume.
- **Priority:** Must-have

### Web

#### FR-004 [EARS-E]: Keyless web search
> WHEN `web.search`, `web.news` or `web.suggestions` is called THE server SHALL query DuckDuckGo, a keyless service, and return the results without requiring an API key.

- **Inputs:** the query; `max_results`.
- **Outputs:** results carrying title, URL and snippet for search and news, and completion strings for suggestions.
- **Business Rules:** `max_results` is capped at 50 regardless of the request. `web.safe_search` and `web.request_timeout_secs` come from config. No API key is needed, which is why this family can be keyless and still useful.
- **Priority:** Must-have

#### FR-005 [EARS-O]: Fetch can bypass the context window
> IF `web.fetch` receives both `mount_id` and `save_path` THEN the server SHALL write the fetched content directly into that volume instead of returning it.

- **Inputs:** the URL, optionally `mount_id` and `save_path`.
- **Outputs:** the content, or the written file.
- **Business Rules:** this context bypass exists for large pages and for cases where the raw HTML is needed, both wasteful or lossy through an LLM context. `save_path` is an absolute POSIX path within the volume.
- **Priority:** Must-have

#### FR-006 [EARS-O]: The two save parameters are all-or-nothing
> IF exactly one of `mount_id` and `save_path` is provided THEN the server SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `mount_id and save_path must both be provided, or neither`.

- **Inputs:** the two optional parameters.
- **Outputs:** the refusal.
- **Business Rules:** a half-specified destination is a caller mistake with two plausible readings, so it is refused rather than guessed.
- **Priority:** Must-have

#### FR-007 [EARS-U]: Download always writes to a volume
> `web.download` SHALL always write its content into a volume and SHALL NOT return the bytes to the caller.

- **Inputs:** the URL, `mount_id`, the destination path.
- **Outputs:** the written file.
- **Business Rules:** binary data such as images, PDFs and archives cannot pass through an LLM context, so returning it is not an option. This is why `mount_id` is required here and optional on `web.fetch`.
- **Priority:** Must-have

### Documentation

#### FR-008 [EARS-E]: Library identity resolution
> WHEN `context7.resolve_library_id` is called THE server SHALL resolve a library name to its Context7 identifier.

- **Inputs:** `library_name`, for example `react`, `tokio` or `numpy`.
- **Outputs:** the identifier.
- **Business Rules:** the call is documented as the step to take before fetching docs. The API is public and needs no key.
- **Priority:** Must-have

#### FR-009 [EARS-E]: Documentation retrieval
> WHEN `context7.get_library_docs` is called THE server SHALL fetch that library's documentation from Context7 and return it as Markdown.

- **Inputs:** the library identifier.
- **Outputs:** Markdown documentation.
- **Business Rules:** the endpoint base is `https://context7.com/api` by default with a 30 second timeout. Markdown is returned rather than HTML because the consumer is an LLM.
- **Priority:** Must-have

### User databases

#### FR-010 [EARS-E]: Extract, operate, commit
> WHEN any `sqlite.*` tool runs THE server SHALL read the database bytes from the volume, write them to a temporary file, open the connection on that file, run the operation, and for a write read the modified bytes back and commit them through the engine.

- **Inputs:** `mount_id`, the `.db` path, the operation.
- **Outputs:** the result, and for writes the committed file.
- **Business Rules:** the temporary file exists because rusqlite needs a real POSIX path and volume content is not on the filesystem. The temporary file is deleted automatically when dropped. The commit path is `fs_ops::write_bytes`, so quota, ACL and audit are enforced (FR-002).
- **Priority:** Must-have

#### FR-011 [EARS-U]: One writer per database
> Concurrent access to the same `(mount_id, db_path)` SHALL be serialized.

- **Inputs:** concurrent requests.
- **Outputs:** serialized execution.
- **Business Rules:** a process-wide lock map keyed by the pair means two requests cannot corrupt the same database. The lock is held for the whole extract-operate-commit cycle, not just the statement. This is a per-process lock, so it does not protect against two replicas.
- **Priority:** Must-have

#### FR-012 [EARS-O]: Query is read-only by statement inspection
> IF the statement passed to `sqlite.query` does not begin with `SELECT` or `WITH` THEN the server SHALL refuse it with the message `sqlite.query only accepts SELECT or WITH statements; use sqlite.execute for writes`.

- **Inputs:** the SQL text.
- **Outputs:** the refusal.
- **Business Rules:** the check is on the first keyword. It separates the read tool from the write tool so an agent's intent is explicit, and it is a usability guard rather than a security boundary: `sqlite.execute` exists and is equally available to the same caller.
- **Priority:** Must-have

#### FR-013 [EARS-E]: Schema introspection
> WHEN `sqlite.list_tables`, `sqlite.describe_table` or `sqlite.list_indexes` is called THE server SHALL report the database's structure.

- **Inputs:** the `.db` path; optionally a table name to filter indexes.
- **Outputs:** tables and views, column and index descriptions.
- **Business Rules:** `list_tables` reads `sqlite_master` for types `table` and `view`, ordered by name. Omitting the table filter on `list_indexes` returns every index.
- **Priority:** Must-have

#### FR-014 [EARS-E]: CSV import, export and maintenance
> WHEN `sqlite.import_csv`, `sqlite.export_csv` or `sqlite.vacuum` is called THE server SHALL perform the operation and commit any resulting change through the engine.

- **Inputs:** the `.db` path plus the CSV path or table name.
- **Outputs:** the modified database or the exported file.
- **Business Rules:** all three follow the extract-operate-commit pattern, so all three charge the quota for the bytes they write back.
- **Priority:** Must-have

#### FR-015 [EARS-U]: Result and time caps
> `sqlite.query` SHALL return at most `sqlite.max_result_rows` rows and SHALL abort a statement exceeding `sqlite.statement_timeout_secs`.

- **Inputs:** the config values.
- **Outputs:** the capped result.
- **Business Rules:** `max_result_rows` defaults to 1000 with a hard ceiling of 10000; `statement_timeout_secs` defaults to 30. The caps bound both server work and the tokens returned to an agent.
- **Priority:** Must-have

### Analytics

#### FR-016 [EARS-U]: Table name derivation from the file stem
> The table name in a `db.query` statement SHALL be the file's stem, lowercased.

- **Inputs:** the file path.
- **Outputs:** the registered table name.
- **Business Rules:** `/data/Sales_2024.csv` is queried as `FROM sales_2024`. The rule is stated in the tool's own description, because an agent has no other way to know what to write.
- **Priority:** Must-have

#### FR-017 [EARS-E]: Analytics over stored data files
> WHEN a `db.*` tool runs THE server SHALL read the file's bytes from the volume, write them to a temporary file, register it with DataFusion, and run the operation.

- **Inputs:** `mount_id`, the file path, the operation.
- **Outputs:** rows, a schema, a profile, or a converted file.
- **Business Rules:** supported formats are CSV, Parquet and JSON. `db.convert` writes its output back through `fs_ops::write_bytes` (FR-002).
- **Priority:** Must-have

#### FR-018 [EARS-O]: File size limit
> IF a data file is larger than `db.max_file_bytes` THEN the server SHALL refuse the operation.

- **Inputs:** the file's size from `stat`.
- **Outputs:** the refusal.
- **Business Rules:** the default is 100 MiB. The size is checked before the bytes are pulled, so an oversized file costs a stat rather than a read.
- **Priority:** Must-have

#### FR-019 [EARS-U]: Row caps
> `db.query` and `db.sample` SHALL return at most `db.max_result_rows` rows.

- **Inputs:** the requested `max_rows`, default 100 for query and capped at 1000 for sample.
- **Outputs:** the capped result.
- **Business Rules:** `max_result_rows` defaults to 1000 with a hard ceiling of 10000. A caller's request is capped rather than refused.
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
- Every family caps its output: 50 web results, 1000 rows by default with a 10000 ceiling for both database families (FR-004, FR-015, FR-019).
- `db.max_file_bytes` bounds the bytes any single analytics call materializes (FR-018).
- The extract-operate-commit pattern copies a whole database file per call, so cost grows with file size rather than with result size. For a large database this is the dominant cost and there is no incremental path.
- External calls carry timeouts: `web.request_timeout_secs` and `context7.request_timeout_secs`, both defaulting to 30 for Context7.

### 7.2 Security
- Every volume write passes the ACL, quota and audit rules (FR-002).
- The per-database lock prevents two requests corrupting one file (FR-011), within one process.
- The `SELECT`-only gate on `sqlite.query` is a usability guard, not a security boundary (FR-012).
- Enabling a family is an operator decision about what the server reaches; all four are off by default (FR-001).
- These tools issue outbound requests to public services, a deployment consideration for a network-restricted environment.

### 7.3 Usability
- `web.download` writing to a volume rather than returning bytes matches what an LLM can actually consume (FR-007).
- The half-specified destination is refused with a message naming both parameters (FR-006).
- The table-naming rule is stated in the tool description, because an agent cannot infer it (FR-016).
- `sqlite.query` refusing a write names the tool to use instead (FR-012).

### 7.4 Reliability
- Temporary files are deleted on drop (FR-010).
- The lock is held across the whole cycle, so a crash between read and commit leaves the stored file unchanged rather than half-written (FR-011).
- Capped results keep one call from exhausting memory.

### 7.5 Observability
Writes produce audit entries like any other write, so a file created by `web.download` or `db.convert` is traceable through `fs.audit_log`. Outbound request duration and failure rate are not measured, so a degraded external service is visible only as slow tool calls.

### 7.6 Deployment
- All four off by default; enable per family in YAML or with `--web`, `--context7`, `--sqlite`, `--db`.
- `web` needs outbound access to DuckDuckGo; `context7` to `https://context7.com/api`.
- Neither needs an API key.
- `sqlite` and `db` need only temporary-directory space proportional to the files they touch.

### 7.7 Scalability
- The per-database lock is per process, so two replicas can write the same stored database concurrently and the last commit wins.
- Temporary file space scales with concurrent calls times file size.
- Outbound rate limits on the public services are not managed by the server; a burst of `web.search` calls is passed straight through.

## 8. E2E Tests

**Network policy.** No test in this suite contacts DuckDuckGo or Context7. Web and documentation tests run against a local stub HTTP server; the live services are exercised only by manual smoke testing.

**Fixtures:** project `spec-int` with `ALICE` as member and `BOB` as a non-member; `/data/Sales_2024.csv` with a header and three rows; `/data/big.csv` of 150 MiB; `/db/app.db` with one table `users` holding two rows; a stub HTTP server returning fixed search results and Markdown docs.

| Test ID | Category | Scenario | FR refs |
|---|---|---|---|
| E2E-001 | Core Journey | SC-001 | FR-004, FR-005 |
| E2E-002 | Side Effect | SC-001 | FR-002, FR-005, FR-007 |
| E2E-003 | Error | SC-001 | FR-003, FR-006 |
| E2E-004 | Error | SC-001 | FR-004, FR-007 |
| E2E-005 | Edge | SC-001 | FR-004, FR-006 |
| E2E-006 | Edge | SC-001 | FR-005, FR-006, FR-007 |
| E2E-007 | Core Journey | SC-002 | FR-008, FR-009 |
| E2E-008 | Error | SC-002 | FR-008 |
| E2E-009 | Error | SC-002 | FR-009 |
| E2E-010 | Edge | SC-002 | FR-008, FR-009 |
| E2E-011 | Core Journey | SC-003 | FR-010, FR-013 |
| E2E-012 | Feature | SC-003 | FR-011, FR-014 |
| E2E-013 | Error | SC-003 | FR-010, FR-012 |
| E2E-014 | Data Integrity | SC-003 | FR-011 |
| E2E-015 | Error | SC-003 | FR-013, FR-015 |
| E2E-016 | Error | SC-003 | FR-014, FR-015 |
| E2E-017 | Edge | SC-003 | FR-010, FR-012, FR-013 |
| E2E-018 | Edge | SC-003 | FR-011, FR-014 |
| E2E-019 | Edge | SC-003 | FR-012, FR-015 |
| E2E-020 | Core Journey | SC-004 | FR-016, FR-017 |
| E2E-021 | Feature | SC-004 | FR-017, FR-019 |
| E2E-022 | Error | SC-004 | FR-016, FR-018 |
| E2E-023 | Error | SC-004 | FR-017, FR-019 |
| E2E-024 | Error | SC-004 | FR-018 |
| E2E-025 | Edge | SC-004 | FR-016, FR-019 |
| E2E-026 | Edge | SC-004 | FR-018 |
| E2E-027 | Core Journey | SC-005 | FR-001 |
| E2E-028 | Error | SC-005 | FR-001 |
| E2E-029 | Security | SC-005 | FR-002, FR-003 |
| E2E-030 | Security | SC-005 | FR-003 |
| E2E-031 | Edge | SC-005 | FR-001 |
| E2E-032 | Edge | SC-005 | FR-002 |

### Key test narratives

**E2E-001 — Search returns titled results against a stub.** Given a stub returning two fixed results, `web.search` with query `rust async` returns two results each carrying a title, URL and snippet; `web.fetch` on the first result's URL returns its body content.

**E2E-002 — A saved fetch is charged and audited like any write.** `web.fetch` with `mount_id`/`save_path` produces a file whose bytes match the fetch, plus one `fs.audit_log` entry; a `web.download` of a binary fixture also produces an audit entry; a save exceeding `safety.write_quota_bytes` fails with `ERR_WRITE_QUOTA_EXCEEDED` and writes nothing.

**E2E-003 — Download requires a destination.** `web.download` with a URL and no `mount_id` fails `ERR_INVALID_ARGUMENT` and never returns the binary payload; with a valid `mount_id` and destination it succeeds and returns a path, not bytes.

**E2E-004 — Result count is capped at 50.** A stub able to return 100 results, requested at `max_results=100`, yields at most 50 results and the call still succeeds.

**E2E-005 — Save parameters are all-or-nothing in both directions.** `mount_id` alone, or `save_path` alone, both fail `ERR_INVALID_ARGUMENT` with the exact message; neither returns content to the caller; both writes and does not return.

**E2E-006 — Library resolution then documentation.** A stub resolving `tokio` to a fixed id; `context7.resolve_library_id` returns it; `context7.get_library_docs` with that id returns Markdown text.

**E2E-007 — An unknown library is not a server error.** A stub returning no match for a nonsense name; `context7.resolve_library_id` succeeds with an empty/unresolved result rather than `ERR_INTERNAL_ERROR`.

**E2E-008 — An unreachable API times out cleanly.** `api_url` pointed at a port that accepts and never answers, `request_timeout_secs=2`; `context7.get_library_docs` fails within 5 seconds and `/health` remains responsive afterward.

**E2E-009 — Documentation tools touch no volume.** Neither `context7.*` schema declares `mount_id`; calling them with no project membership anywhere still succeeds.

**E2E-010 — Concurrent writes to one database are serialized.** Ten concurrent `sqlite.execute` calls each incrementing a counter row by 1; afterwards `sqlite.query` reports 10 with no lost update, and `sqlite.list_tables` still succeeds (no corruption).

**E2E-011 — Row results are capped.** A table of 5000 rows, `sqlite.max_result_rows=100`; `sqlite.query SELECT *` returns at most 100 rows and `row_count` matches; the call succeeds.

**E2E-012 — Import and export round-trip through the engine.** `sqlite.import_csv` from a 3-row CSV into a new table; `sqlite.query` returns three rows; the write appears in `fs.audit_log`; `sqlite.export_csv` writes the table back out and that write is also audited.

**E2E-013 — The read tool refuses writes and the write tool accepts them.** `sqlite.query` with `DELETE FROM users` fails with the documented message and `users` is unchanged; the same statement via `sqlite.execute` succeeds; a `WITH ... SELECT` is accepted by `sqlite.query`.

**E2E-014 — A read leaves the stored file byte-identical.** `fs.hash` recorded before and after each of `sqlite.query`/`list_tables`/`describe_table`/`list_indexes` is identical and no audit entry is written; after `sqlite.vacuum` the hash differs and an audit entry exists.

**E2E-015 — Statement timeout aborts a long query.** `statement_timeout_secs=1`; a long-running recursive CTE via `sqlite.query` fails rather than hanging, returns within a few seconds, and a subsequent quick query against the same database succeeds (lock released).

**E2E-016 — An unknown table name is a clear error.** `db.query SELECT * FROM sales` on `/data/Sales_2024.csv` fails naming the unknown table; `FROM sales_2024` succeeds with columns matching the CSV header.

**E2E-017 — Row caps apply to query and sample.** `db.max_result_rows=50` on a 5000-row CSV; `db.query max_rows=4000` and `db.sample rows=4000` each return at most 50 rows and both calls succeed.

**E2E-018 — An oversized file is refused before it is read.** `/data/big.csv` at 150 MiB against the 100 MiB default; `db.schema` fails fast naming the size limit; the same call on a 1 MiB file succeeds.

**E2E-019 — Each supported format is readable and convertible.** The same data as CSV, Parquet and JSON report identical columns/types via `db.schema`; `db.convert` CSV→Parquet produces a file `db.schema` reads with the same columns, and that write is audited.

**E2E-020 — The size check precedes the format check.** A 150 MiB file with an unsupported extension; `db.schema` fails naming the size limit, not the unsupported format.

**E2E-021 — A disabled family's tools are absent, not failing.** With all four families disabled, `tools/list` shows no `web.`/`context7.`/`sqlite.`/`db.` name and the always-on total; `tools/call web.search` returns a JSON-RPC unknown-tool error.

**E2E-022 — A non-member cannot write through an integration.** `BOB`, not a member of `spec-int`: `web.download`, `sqlite.query`, and `db.schema` against `spec-int` all fail `ERR_FORBIDDEN`; nothing is written or read.

**E2E-023 — A platform admin gains no access through an integration.** `ADMIN`, a platform admin and not a member of `spec-int`: `sqlite.list_tables` and `db.sample` fail `ERR_FORBIDDEN`; `ADMIN` can still call `admin.list_all_projects`.

**E2E-024 — Enabling one family does not enable the others.** A server started with only `sqlite` enabled exposes exactly 8 `sqlite.` tools and none from the other three families.

**E2E-025 — Every integration write lands in the audit log.** All four families enabled; a `web.download`, a `sqlite.execute`, a `sqlite.export_csv` and a `db.convert` each produce one `fs.audit_log` entry with a byte count, and the session's quota advances by the total.

## 9. Glossary

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

## 10. Confidence Notes

- **High overall confidence.** The pre-move source spec's functional claims (gating, extract-operate-commit, the lock, the statement gate, the table-naming rule, the caps) were spot-checked against the current source at `crates/core/src/tools/{web,context7,sqlite,db}.rs`, `crates/core/src/tools/all.rs` and `crates/core/src/config.rs`: all logic, error messages and defaults match verbatim.
- **Path drift (cosmetic, not a functional finding).** The pre-move spec cites `crates/mcp-fs/src/tools/*.rs`. The current tree (per AGENTS.md and verified directly) places this logic at `crates/core/src/tools/*.rs`; `crates/mcp-fs` is now the thin binary crate. All paths in this document use the current location.
- **Line-number drift (cosmetic).** Several cited line numbers (e.g. `tools/all.rs:37-43`) have shifted by a few lines as the file grew; the referenced code and behaviour are unchanged. Exact line numbers are intentionally omitted from this functional spec; design.md's requirement-to-code map carries current citations.
- No functional drift found between the pre-move spec and the as-built code; the four families, their tool counts (5/2/8/5), defaults, error messages, and caps all verified unchanged.
