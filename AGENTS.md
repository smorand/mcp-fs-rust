# mcp-fs (Rust)

## Overview
A **streamable-HTTP MCP server** exposing a **simulated
multi-project filesystem** (97 tools: 35 `fs.*`, 13 `admin.*`, 39 `git.*`, 4 `git.auth*`, 6 `git.pr_*`;
+ 4 `search.*` when search enabled),
a REST data plane at `/api/fs` with OpenAPI at `/api/swagger.json` and Swagger UI at
`/api/docs`, and an optional Git HTTP smart server at `/git/{mount_id}/` with push,
fetch and pull against any declared host (including GitHub Enterprise), a per-host
token store, a browser token screen at `/app/tokens`, and a browser screen at
`/app/deleted-projects` listing soft-deleted projects with an undelete action. Ships with `agent`, an
interactive CLI agent that drives the tools through an LLM (`./agent.sh`).

Server state (metadata tree, ACL, git index, OAuth tokens) lives in a relational store:
**SQLite** by default, **PostgreSQL** or **SQL Server** per store via config. Blob bytes
stay in `infra.blob` (local or S3).

An optional `doc_service` section plugs an external document to Markdown converter in,
either a CLI (`doc-convert --stdout {document}`, sandboxed in a per call tempdir) or an
HTTP endpoint. It is off by default and drives `fs.write_bytes`'s
`trigger_documentation_service` flag, the same flag on the REST `/upload`, and
`fs.documentize`. The companion is written at the path `fs.extract_text` already uses.

Started as a strict 1:1 port of a C# implementation. **That is history**: the C# is not a
reference, must not be read, and is diverging by design (it has no PostgreSQL support,
which is not its target). This project's own tool contract is authoritative:
`TOOL_CONTRACT.txt` for humans, `tool-contract-golden.json` machine checked on every test
run. Lineage and design decisions: `.agent_docs/lineage.md`.

Stack: Rust 2024, axum + tokio, rmcp 3.5.0 (`server` + `transport-streamable-http-server`
features only, the one production MCP transport, SPEC-0013), rusqlite (bundled), sqlx (optional,
PostgreSQL), tiberius-ng + bb8 (optional, SQL Server), aws-sdk-s3, jsonwebtoken + rsa, git2
(libgit2), tree-sitter, pdf-extract, quick-xml, zip, aes-gcm, reqwest, clap, tracing.

Cargo features, none in `default`: `postgres`, `sqlserver`, `rag`, `all-backends`. A default build
is SQLite only and carries neither relational driver. The `rag` feature enables pgvector (PostgreSQL)
and sqlite-vec (SQLite) for vector search; it implies `postgres`.

## Key commands
```
make build | make test | make run       same as ./build.sh / ./test.sh / ./run.sh
make check                            format-check + lint + typecheck + security + test
make test-e2e-full                    real PostgreSQL + MinIO + git CLI, full lifecycle (mandatory infra, see .agent_docs/testing.md)
make help                             list every target
./build.sh                            cargo build --release
./test.sh                             cargo test --workspace
./run.sh                              .env + config bootstrap + keys + build + serve :5002
cargo clippy --all-targets --all-features -- -D warnings   quality gate, must be clean
mcp-fs serve | keys | token | migrate | purge | version
mcp-fs migrate --from a.yaml --to b.yaml  offline copy of relational state between backends
mcp-fs purge [--project <id> [--on-demand]]  run the auto-purge sweep once; no --project runs
                                      every configured project plus the grace-period removal
docker compose -f docker-compose.test.yml up -d  postgres + mssql for the opt-in suites
MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current
                                      regenerate the frozen tool contract, then review the diff
python3 scripts/doc_service_fake.py --port 8099   fake document service, for doc_service api mode
./agent.sh --user <name>              interactive CLI agent; starts/stops the server if needed
python3 scripts/pty_check.py          agent line editor checks on a real pty
```

## Project structure
All logic lives in the library crate `crates/core` (package `mcp-fs-core`, crate name
`mcp_fs_core`; the directory is `crates/core`, not `crates/mcp-fs-core`, to match the skill's
mandated layout while keeping the Cargo package/crate name off the language's own `core`
sysroot crate). `crates/mcp-fs` keeps only `src/main.rs` (9 lines) and a path dependency on
`mcp-fs-core`; its `postgres`/`sqlserver`/`rag`/`all-backends` features forward to the same
features on `mcp-fs-core`. The paths below are all under `crates/core/src/` unless noted.
- `cli.rs` : clap verbs (serve/keys/token/migrate/purge/version), config path resolution.
- `purge.rs` : the auto-purge engine — `sweep_project_files`/`sweep_project`/`sweep_grace_period`,
  each driven by `atime`/`created_at`/`deleted_at` against the per-project `PurgeConfig` and the
  global `project_purge_grace_days`; `run_cycle` is the one entry point both the background loop
  (`app.rs`) and the CLI `purge` verb call, so neither reimplements the sweep (SPEC-0014).
- `app.rs` : axum Router assembly, shared state, the MCP route (an `rmcp` `StreamableHttpService`
  wrapping `mcp::server::McpServer`, `LocalSessionManager` + `legacy_session_mode: true` so
  `initialize` is mandatory; see `.agent_docs/architecture.md` for the exact wire shapes), `/health`.
- `mcp/` : `server.rs` only — `McpServer`, one `#[tool]` method per entry in `TOOL_CONTRACT.txt`
  (97 tools, `#[tool_router]`), each calling the same `core::fs_ops`/engine function the REST plane
  calls, never reimplementing an operation. The hand-rolled JSON-RPC/SSE framing layer this module
  used to own (`ToolRegistry`/`ToolSchema`/`Args`/`ToolHandler`) is gone (SPEC-0013); what remains
  of that machinery lives at `tools::registry_support`, kept only because `Args`/`ToolCtx` are the
  test-dispatch types for the five optional families' `#[cfg(test)]`-gated `register()` and the
  shared test harness, and `ToolRegistry`/`ToolSchema` back the five optional families'
  (`web`/`context7`/`sqlite`/`db`/`doc`) production `catalog()` read by `/api/swagger.json`.
- `config.rs` : full `ServerConfig` + `${VAR}` expansion + `Dsn` (redacts in Debug and
  Display) + boot validation of every `infra.*` store.
- `errors.rs` : the 14 `ERR_*` codes, `ToolError`, HTTP status mapping, plus a `retryable`
  flag for transient database failures (no new code).
- `migrate.rs` : the `migrate` verb, offline row for row copy between backends.
- `identity.rs` : RS256 verification, 30s clock skew, forwarded header then `Authorization`.
- `safety.rs` : path normalization, read-before-write, write quota, capped audit, trash.
- `storage/` : `rel/` (the relational layer: `RelationalDb`/`RelationalTx` owned handle,
  `dialect.rs`, `schema.rs`, `sqlite.rs`, `postgres.rs`, `sqlserver.rs`, `run_retrying`),
  `sqlite.rs` (serialized, WAL, offloaded), `meta.rs` (nodes + blob_refs,
  content-addressing, refcount GC), `admin.rs` (project/project_member, caseless ACL,
  project id validation), `blob/local.rs` + `blob/s3.rs`, `volume.rs` (VolumeClient),
  `conformance.rs` (one suite, every engine), `mod.rs` (factories, `RelationalRegistry`
  pool cache, StoreManager).
- `core/` : `fs_ops.rs` (every filesystem operation, the engine the tools call), `diff.rs`.
- `docs/` : `extract.rs`, `docx.rs`, `symbols.rs` (tree-sitter + lexical fallback),
  `ocr.rs` (pluggable, null by default), `mime.rs`, `service.rs` (the external
  document to Markdown converter, cli or api, off by default).
- `tools/` : one module per family. Every family exposes typed `pub(crate)` async functions
  (the real logic, called directly by `mcp::server::McpServer`'s `#[tool]` methods and by the
  REST plane); `register(&mut ToolRegistry)` survives only as `#[cfg(test)]`-gated glue for the
  shared test harness, except the five optional families (`web`/`context7`/`sqlite`/`db`/`doc`,
  off by default, not part of the 97-tool contract), whose `register()`/`catalog()` are genuine
  production code backing `/api/swagger.json`. `catalog.rs` holds the transport-agnostic
  `ToolCatalogEntry` shape; `all.rs` has the now-test-only `register_all`.
- `api/` : `dataplane.rs` (the `/api/fs` routes), `openapi.rs` (spec + Swagger UI).
- `token_screen.rs` : the `/app/tokens` browser screen (git enabled only), session-cookie +
  CSRF, delegating to `tools::git_auth`.
- `deleted_projects_screen.rs` : the `/app/deleted-projects` browser screen (always mounted),
  the same session-cookie + CSRF convention, delegating to `tools::admin::{list_deleted_projects,
  undelete_project}` — never a second implementation (SPEC-0014 US-0012).
- `git/` : `db.rs` (SQLite index, plus the `git_operations` row that persists a paused
  combine operation), `merge.rs` (the shared merge engine: one three way merge, one atomic
  volume apply, one quota charge, and the response TYPES every combine operation
  serializes through, so a wire shape disagreement fails to compile),
  `odb.rs` (objects in the blob store under `git:{sha}`),
  `repo.rs` (per project repository + write lock), `http/` (pkt-line, upload-pack,
  receive-pack), `oauth/` (store, AES-GCM cipher, encrypted persistence, device flow),
  `provider/` (the pull request seam: host to provider and API base resolution, the
  injectable `ProviderClient`, the transport safety rules, and `model.rs`, the ONE
  normalized pull request object every `git.pr_*` tool returns).
- `crates/agent/` : the interactive CLI agent (an MCP **client**, not part of the server).
  `mcp.rs` (stateless JSON-RPC, fuzzy tool name resolution), `llm.rs` (OpenAI compatible
  streaming with tool calling), `input.rs` (wrap aware line editor), `ui.rs` (markdown to
  ANSI), `spinner.rs`, `session.rs`. Config: `config/agent_test.yaml`.
- `TOOL_CONTRACT.txt` : the 97 tool schemas and return shapes, human readable. **This is
  the authoritative contract.**
- `tool-contract-golden.json` : the same contract, machine checked. Three tests compare
  every name, description and `inputSchema` against it, serialized, so a reordered schema
  key fails too. Never hand edited: regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1` and
  review the diff (`tools/contract_golden.rs`).

## Conventions
- `mount_id` is a required parameter on every `fs.*` and `git.*` tool. Errors are
  `ToolError::<code>(msg)` carrying a stable `ERR_*`.
- Tool parameter names are snake_case, which is what the generated JSON schema exposes.
  Parameter descriptions are the LLM-facing docs and are frozen: copy them verbatim from
  `TOOL_CONTRACT.txt`, and treat any edit as a deliberate contract change.
- Every `fs.*`/`git.*` handler: `state.authorize(mount_id, person)` first, then normalize
  every path through `state.safety.normalize_path`, then call the engine in `core::fs_ops`.
  Never reimplement an operation in the tool layer, and never in `api/dataplane.rs`
  either: the MCP surface and the REST plane MUST share one implementation, otherwise a
  fix on one path silently leaves the other on the old behaviour. `core::fs_ops` is the
  only place an operation is written; both layers are thin adapters over it.
- Platform admin manages projects and membership; it does NOT get implicit file access.
  Keep that separation.
- A store NEVER speaks a driver: it speaks `storage::rel::RelationalDb`. A transaction is
  an owned handle from `begin()`, not a closure. `run_retrying` re runs its closure, so
  calling it asserts the work is idempotent; use `begin()` for anything touching state
  outside the transaction. Never block a request thread on the database.
- `volume_id` scopes every row of `nodes`, `blob_refs` and `git_*`, because one PostgreSQL
  or SQL Server database holds every volume. It belongs in EVERY `WHERE` clause: omitting
  it leaks or corrupts another volume's rows.
- A `LIKE` pattern is built with `descendant_pattern` / `Dialect::escape_like_literal`,
  never by hand. An unescaped prefix made `a_b` match `axb`, so a subtree delete removed
  unrelated rows.
- Files are content-addressed: write puts the blob then the node, copy is metadata only,
  delete GCs the blob at refcount 0, an empty file stores no blob. The local layout
  `{root}/{sha[..2]}/{sha}` is part of the on-disk contract.
- Secrets come from the environment only (`${VAR}` expansion in YAML, or read at the
  composition root). Never log a token or a key.
- Comments explain WHY. No dashes as punctuation anywhere in code, comments or output.
- Adding a blob backend: implement the trait in `storage/traits.rs` plus a branch in
  `storage/mod.rs`. Adding a relational backend: see `.agent_docs/backends.md` for the
  dialect checklist.
- Glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.
  It is Python `fnmatch` semantics, so a single `*` DOES cross a '/' boundary (verified
  against the reference: `*.rs` matches `/src/nested.rs`). Do not reach for `globset` with
  `literal_separator`, it would silently narrow `fs.glob` and `fs.grep`.

## Quality gate
`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings` and
`cargo fmt --all -- --check` must all be clean before any commit. `--all-features` matters:
without it the two optional drivers are never compiled.

`cargo fmt --all` is SAFE to run: the tree is kept clean against `rustfmt.toml`, which sets
`use_small_heuristics = "Max"` to encode the wide house style (calls and chains stay on one
line rather than exploding into one argument per line). Before that file existed the repo
was not fmt clean and running fmt rewrote 84 files, so do not reintroduce hand formatting
that fights it.

## Behaviour worth knowing
Each is documented at its call site and recorded in `.agent_docs/lineage.md`: `unpack ok`
in the push report, membership enforced on git routes, `max_pack_size_mb` enforced, pushed
objects really indexed, no custom libgit2 ODB backend (blob store is the source of truth,
bytes identical), `ERR_NOT_SUPPORTED` for an unsupported extraction format, numbered list
markers kept in generated docx, `volume_id` multi tenancy, dialect binary typing for the
OAuth token, a merge conflict being an `Ok` response carrying `status: "conflict"` rather
than an error (with every combine response serialized through the types in
`git/merge.rs`, never a hand built `json!`), and `TextKey(n)` bounded keys (SQL Server cannot index `NVARCHAR(MAX)`, so
keyed text has a length ceiling there).

## Backlog
The backlog is the `backlog/` directory: one `BL-NNNN_slug.md` file per item.

## Documentation index
- `.agent_docs/architecture.md` : storage model, request lifecycle, safety, error logging.
- `.agent_docs/tools.md` : the 97 tool reference (families, parameters, authorization).
- `.agent_docs/api.md` : the `/api/fs` REST plane and the OpenAPI single source of truth.
- `.agent_docs/git.md` : git objects in the blob store, HTTP smart protocol, the
  `git.hosts` host map, OAuth/PAT tokens (per person+host), the remote pipeline
  (URL validation, credential supply, clone/push/fetch/pull), the `/app/tokens` screen,
  the `git.pr_*` surface and the normalized pull request model.
- `.agent_docs/config.md` : full YAML schema, backends and dsn, resolution order, secrets.
- `.agent_docs/backends.md` : the relational layer, adding a backend, dialect checklist,
  and the SQL Server driver decision record (read before touching `rel/sqlserver.rs`).
- `.agent_docs/testing.md` : test layout, PostgreSQL/SQL Server and MinIO opt-in suites.
- `.agent_docs/lineage.md` : the C# lineage, why it is no longer a reference, design decisions.
- `.agent_docs/agent.md` : the CLI agent, its config, and the terminal invariants it depends on.
- `.agent_docs/search.md` : search tools, per-project index modes and auto indexing,
  BM25/RAG/reranking config, per-backend caveats.
