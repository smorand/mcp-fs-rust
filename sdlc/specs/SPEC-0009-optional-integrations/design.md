> Id: SPEC-0009
> Nature: FEAT
> Status: as-built
> Area: optional-integrations
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# mcp-fs Optional Integrations — Design

## 1. Components

| Component | File | Role |
|---|---|---|
| `web` family | `crates/core/src/tools/web.rs` | 5 tools: `search`, `news`, `suggestions`, `fetch`, `download`, against DuckDuckGo (keyless). |
| `context7` family | `crates/core/src/tools/context7.rs` | 2 tools: `resolve_library_id`, `get_library_docs`, against `https://context7.com/api` (keyless). |
| `sqlite` family | `crates/core/src/tools/sqlite.rs` | 8 tools over user `.db` files stored in a volume, via rusqlite on a materialized temp file, serialized by a process-wide lock map. |
| `db` family | `crates/core/src/tools/db.rs` | 5 tools over CSV/Parquet/JSON files, via DataFusion registered against a materialized temp file; table name is the file stem lowercased. |
| Registration | `crates/core/src/tools/all.rs` | `register_all` branches on `features.{web,context7,sqlite,db}`, each calling the family's `register(reg, &config.<family>)` only when its flag is set. |
| Config | `crates/core/src/config.rs` | `WebConfig`, `Context7Config`, `SqliteConfig`, `DbConfig` structs; all four `enabled` default `false`. |
| Engine seam | `crates/core/src/core/fs_ops.rs` | The only place a volume write/read is implemented; every family's write path (`web.fetch`/`download` saves, `sqlite.*` commits, `db.convert`) calls into it, never reimplementing ACL/quota/audit. |

## 2. Flows

### 2.1 Web fetch/save (FR-005, FR-006, FR-007)
1. `web.fetch(url, mount_id?, save_path?)`.
2. If exactly one of `mount_id`/`save_path` is set → `ToolError::InvalidArgument("mount_id and save_path must both be provided, or neither")`.
3. If neither set → HTTP GET, return body text/content to caller (no volume touched).
4. If both set → HTTP GET, then `fs_ops::write_bytes(mount_id, save_path, bytes, caller)` → ACL + quota + audit, as `fs.write`.
5. `web.download(url, mount_id, path)` is step 4 unconditionally; it never returns bytes.

### 2.2 Context7 lookup (FR-008, FR-009)
1. `context7.resolve_library_id(library_name)` → HTTP GET against `api_url`, returns resolved id or an empty/unresolved result (never a server error for "not found").
2. `context7.get_library_docs(library_id)` → HTTP GET, Markdown body returned as-is. No volume/mount_id involved; no membership check applies to this family.

### 2.3 SQLite extract-operate-commit (FR-010, FR-011, FR-012)
1. Acquire the process-wide lock for `(mount_id, db_path)` (blocks until free).
2. Read `.db` bytes from the volume via the engine's read path (enforces membership via FR-003).
3. Write bytes to a fresh temp file (`tempfile`-style, real POSIX path required by rusqlite).
4. Open rusqlite connection on the temp path; run the statement (`sqlite.query` gated to `SELECT`/`WITH` by first-keyword inspection; all other `sqlite.*` tools, including `sqlite.execute`, unrestricted).
5. For a write tool (`execute`, `import_csv`, `vacuum`, and the modified side of `export_csv`'s source db if applicable): read the temp file's bytes back, commit via `fs_ops::write_bytes` (ACL + quota + audit).
6. Temp file dropped (deleted) when it goes out of scope.
7. Release the lock.

### 2.4 DataFusion analytics (FR-016, FR-017, FR-018, FR-019)
1. `stat` the target file; if size > `db.max_file_bytes` → refuse before reading bytes.
2. Read bytes from the volume, write to a temp file.
3. Register the temp file with a DataFusion `SessionContext` under the table name = file stem, lowercased.
4. Run the operation (`query`/`schema`/`sample`/`profile`); for `convert`, write the output format to a new temp file, read it back, commit via `fs_ops::write_bytes`.
5. Row-returning operations cap output at `max_rows` requested, bounded by `db.max_result_rows` (default 1000, hard ceiling 10000).

## 3. Interfaces

| Tool | Params (key ones) | Mount required | Writes volume |
|---|---|---|---|
| `web.search` / `web.news` / `web.suggestions` | query, max_results (≤50) | no | no |
| `web.fetch` | url, mount_id?, save_path? | optional, paired | only if both given |
| `web.download` | url, mount_id, path | required | always |
| `context7.resolve_library_id` | library_name | no | no |
| `context7.get_library_docs` | library_id | no | no |
| `sqlite.query` | mount_id, db_path, sql (SELECT/WITH only) | required | no |
| `sqlite.execute` | mount_id, db_path, sql | required | yes |
| `sqlite.list_tables` / `describe_table` / `list_indexes` | mount_id, db_path, table? | required | no |
| `sqlite.import_csv` / `export_csv` / `vacuum` | mount_id, db_path, csv_path/table | required | yes |
| `db.query` | mount_id, file_path, sql, max_rows | required | no |
| `db.schema` / `sample` / `profile` | mount_id, file_path | required | no |
| `db.convert` | mount_id, file_path, out_path, out_format | required | yes |

## 4. Data and State

No persisted server entity; all state is either the external services (stateless to this server) or the caller's own files in the volume.

| Artifact | Shape | Home |
|---|---|---|
| Web result | title, URL, snippet | in-memory, returned |
| Saved page or download | a file in the volume | S1's Volume (blob store + metadata) |
| Library docs | Markdown | in-memory, returned |
| User database | a `.db` file in the volume, owned by the caller | S1's Volume |
| Query result | `{columns, rows, row_count}` | in-memory, returned |
| Data file | CSV, Parquet or JSON in the volume | S1's Volume |
| Per-database lock | process-wide `HashMap<(mount_id, db_path), Mutex<()>>`-style map | `crates/core/src/tools/sqlite.rs`, process memory only, not persisted, not shared across replicas |

## 5. Configuration

| Config | Field | Default | Notes |
|---|---|---|---|
| `WebConfig` | `enabled` | `false` | gate |
| | `safe_search` | (config default) | passed to DuckDuckGo |
| | `request_timeout_secs` | 30-class default | HTTP timeout |
| `Context7Config` | `enabled` | `false` | gate |
| | `api_url` | `https://context7.com/api` | overridable (used by stub-based tests) |
| | `request_timeout_secs` | 30 | HTTP timeout |
| `SqliteConfig` | `enabled` | `false` | gate |
| | `max_result_rows` | 1000 | hard ceiling 10000 |
| | `statement_timeout_secs` | 30 | aborts long statements |
| `DbConfig` | `enabled` | `false` | gate |
| | `max_file_bytes` | 100 MiB | checked via `stat` before read |
| | `max_result_rows` | 1000 | hard ceiling 10000 |

All four `enabled` flags can be forced on by a matching CLI flag (`--web`, `--context7`, `--sqlite`, `--db`) regardless of the YAML value, per the core platform's CLI-override rule.

## 6. Observability

- Every committed write (via `fs_ops::write_bytes`) produces an `fs.audit_log` entry with a byte-count `detail`, identical to any `fs.write`, for `web.fetch`(saved)/`download`, `sqlite.execute`/`import_csv`/`export_csv`/`vacuum`, and `db.convert`.
- Reads (`sqlite.query`, `list_tables`, `describe_table`, `list_indexes`, all `db.*` read tools) produce no audit entry and leave the stored file's hash unchanged.
- Outbound HTTP call duration/failure rate to DuckDuckGo/Context7 is **not** measured or logged structurally; a degraded external service is observable only indirectly, as a slow or failing tool call.

## 7. Decisions

- **DEC-001** (was DEC-801): The four families are specified in one document rather than four. **Rationale:** they share one shape — off by default, gated by a flag, writing back only through the engine — and three of the four are small; four documents would repeat that shape four times and obscure it. **Alternatives considered:** one spec per family. **Implemented by:** FR-001, FR-002, FR-003. **Code evidence:** `crates/core/src/tools/all.rs` (register_all).
- **DEC-002** (was DEC-802): The `SELECT`-only gate on `sqlite.query` is specified as a usability guard, explicitly not a security boundary. **Rationale:** `sqlite.execute` is available to the same caller, so treating the gate as protection would be a false claim. **Alternatives considered:** presenting it as a read-only capability. **Implemented by:** FR-012, §7.2 (spec.md), consistency note. **Code evidence:** `crates/core/src/tools/sqlite.rs` (statement-prefix check alongside the unrestricted `sqlite.execute` registration).
- **DEC-003** (was DEC-803): No test contacts the live public services. **Rationale:** a test depending on DuckDuckGo is a test that fails when DuckDuckGo changes, blocks the CI network, or rate-limits; the resulting red build says nothing about this codebase. **Alternatives considered:** live smoke tests gated on a network flag (tends to become permanently skipped). **Implemented by:** §8 network policy (spec.md). **Code evidence:** `crates/core/src/tools/web.rs` and `crates/core/src/tools/context7.rs` call public endpoints directly, with no built-in stub.
- **DEC-004** (was DEC-804): The per-database lock's process scope is stated as a limitation rather than left implied. **Rationale:** the lock reads as complete concurrency control until asked what happens with two replicas, and the answer is silent data loss. **Alternatives considered:** describing the lock without the caveat. **Implemented by:** FR-011, E2E-010 (was E2E-814). **Code evidence:** `crates/core/src/tools/sqlite.rs`, a process-wide static lock map.
- **DEC-005** (was DEC-805): The table-naming rule is specified as a functional requirement rather than left as documentation only. **Rationale:** an agent cannot infer that `/data/Sales_2024.csv` is queried as `sales_2024`; the rule is part of the callable contract, and a change to it would break every stored prompt relying on it. **Alternatives considered:** leaving it only in the tool description (where it also lives). **Implemented by:** FR-016, E2E-016 (was E2E-822). **Code evidence:** `crates/core/src/tools/db.rs` (table-name derivation + tool description string).

## 8. Requirement to Code Map

| FR | File | Notes |
|---|---|---|
| FR-001 | `crates/core/src/tools/all.rs` | `register_all`, one `if features.<family>` branch per family |
| FR-002 | `crates/core/src/tools/{web,sqlite,db}.rs` + `crates/core/src/core/fs_ops.rs` | every committed write calls `fs_ops::write_bytes` |
| FR-003 | `crates/core/src/tools/{web,sqlite,db}.rs` | `mount_id` param + membership gate; `context7.rs` has neither |
| FR-004 | `crates/core/src/tools/web.rs` | search/news/suggestions against DuckDuckGo, `max_results` capped at 50 |
| FR-005 | `crates/core/src/tools/web.rs` | `fetch` with both params saves to volume |
| FR-006 | `crates/core/src/tools/web.rs` | exact message `"mount_id and save_path must both be provided, or neither"` |
| FR-007 | `crates/core/src/tools/web.rs` | `download` always writes, never returns bytes |
| FR-008 | `crates/core/src/tools/context7.rs` | `resolve_library_id` |
| FR-009 | `crates/core/src/tools/context7.rs` | `get_library_docs`, default `api_url`/timeout in `config.rs` |
| FR-010 | `crates/core/src/tools/sqlite.rs` | extract-operate-commit cycle (module doc comment states the 6 steps) |
| FR-011 | `crates/core/src/tools/sqlite.rs` | process-wide lock map keyed by `(mount_id, db_path)` |
| FR-012 | `crates/core/src/tools/sqlite.rs` | exact message `"sqlite.query only accepts SELECT or WITH statements; use sqlite.execute for writes"` |
| FR-013 | `crates/core/src/tools/sqlite.rs` | `list_tables`/`describe_table`/`list_indexes` against `sqlite_master` |
| FR-014 | `crates/core/src/tools/sqlite.rs` | `import_csv`/`export_csv`/`vacuum` |
| FR-015 | `crates/core/src/config.rs` + `crates/core/src/tools/sqlite.rs` | `SqliteConfig.max_result_rows`/`statement_timeout_secs` |
| FR-016 | `crates/core/src/tools/db.rs` | file-stem-lowercased table name, stated in module doc and tool description |
| FR-017 | `crates/core/src/tools/db.rs` | DataFusion registration over temp file; CSV/Parquet/JSON |
| FR-018 | `crates/core/src/tools/db.rs` | `stat`-based size check before read |
| FR-019 | `crates/core/src/config.rs` + `crates/core/src/tools/db.rs` | `DbConfig.max_result_rows`, per-call `max_rows` |

## 9. Legacy Mapping

Source: `specs/SPEC-0009_2026-09-18_20-35-00-optional-integrations/spec.md` (pre-move).

| Legacy ID | Current ID |
|---|---|
| FR-801 | FR-001 |
| FR-802 | FR-002 |
| FR-803 | FR-003 |
| FR-804 | FR-004 |
| FR-805 | FR-005 |
| FR-806 | FR-006 |
| FR-807 | FR-007 |
| FR-808 | FR-008 |
| FR-809 | FR-009 |
| FR-810 | FR-010 |
| FR-811 | FR-011 |
| FR-812 | FR-012 |
| FR-813 | FR-013 |
| FR-814 | FR-014 |
| FR-815 | FR-015 |
| FR-816 | FR-016 |
| FR-817 | FR-017 |
| FR-818 | FR-018 |
| FR-819 | FR-019 |
| SC-801..SC-805 | SC-001..SC-005 |
| DEC-801..DEC-805 | DEC-001..DEC-005 |
| E2E-801..E2E-832 | E2E-001..E2E-032 (1:1 in original order; E2E-801/803 marked "Existing" became part of E2E-001/E2E-003 equivalents) |
| TBD-801..TBD-804 | Carried forward as open findings, see below |

**Mechanism NFRs carried forward:** extract-operate-commit's whole-file-copy cost (§7.1), per-process lock scope (§7.2/§7.7), unmeasured outbound latency (§7.5), unmanaged outbound rate limits (§7.7) — all unchanged in the as-built system, all still open.

### FINDINGS FOR BACKLOG

1. **TBD-801 (carried, open):** Extract-operate-commit copies the whole database or data file per call. For a large SQLite database every query pays a full read and every write a full read plus a full write; there is no incremental path. No backlog item currently tracks this.
2. **TBD-802 (carried, open):** Outbound request duration and failure rate to DuckDuckGo/Context7 are unmeasured; a degraded public service is visible only as slow tool calls, with no structured signal.
3. **TBD-803 (carried, open):** The per-database lock is per-process. Two replicas can write the same stored database concurrently and the last commit wins, silently losing the other's changes. Relevant if/when this server is deployed with more than one replica.
4. **TBD-804 (carried, open):** Neither `web.*` nor `context7.*` rate-limits its outbound calls; a burst of agent searches passes straight through to a public service that may throttle or block the deployment.
5. **Documentation drift:** the pre-move spec document and inline module paths referred to `crates/mcp-fs/src/tools/*`; the crate was since restructured so this logic lives at `crates/core/src/tools/*`. No functional impact, but any other retained document still citing the old path should be corrected.
