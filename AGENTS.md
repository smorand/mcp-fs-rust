# mcp-fs (Rust)

## Overview
Rust port of the C# `mcp-fs`: a **streamable-HTTP MCP server** exposing a **simulated
multi-project filesystem** (55 tools: 33 `fs.*`, 8 `admin.*`, 11 `git.*`, 3 `git.auth*`),
a REST data plane at `/api/fs` with OpenAPI at `/api/swagger.json` and Swagger UI at
`/api/docs`, and an optional Git HTTP smart server at `/git/{mount_id}/`. Ships with
`agent`, an interactive CLI agent that drives the tools through an LLM (`./agent.sh`).

**Strict 1:1 external parity with the C#** is the prime directive: same tool names,
snake_case parameters, `ERR_*` codes, JSON shapes, SQLite schemas, git wire protocol and
REST routes. A volume written by either implementation must be readable by the other.
Reference implementation: `../mcp-fs-csharp`. Normative spec:
`../mcp-fs-csharp/specs/2026-08-04_09:17:19-rust-port-retrospec.md`.

Stack: Rust 2024, axum + tokio, rusqlite (bundled), aws-sdk-s3, jsonwebtoken + rsa,
git2 (libgit2), tree-sitter, pdf-extract, quick-xml, zip, aes-gcm, reqwest, clap, tracing.

## Key commands
```
./build.sh                            cargo build --release
./test.sh                             cargo test --workspace
./run.sh                              .env + config bootstrap + keys + build + serve :5002
cargo clippy --all-targets -- -D warnings      quality gate, must be clean
mcp-fs serve | keys | token | version
cargo run -p parity-harness -- capture|compare  the 1:1 parity judge
./agent.sh --user <name>              interactive CLI agent; starts/stops the server if needed
python3 scripts/pty_check.py          agent line editor checks on a real pty
```

## Project structure
- `crates/mcp-fs/src/main.rs`, `cli.rs` : clap verbs (serve/keys/token/version), config path resolution.
- `app.rs` : axum Router assembly, shared state, the MCP endpoint, `/health`.
- `mcp/` : the hand-rolled MCP layer. `mod.rs` (JSON-RPC + SSE framing, result/error helpers),
  `registry.rs` (name -> schema + handler, dot/underscore tolerant resolve), `schema.rs`
  (declarative builder reproducing the C# JSON Schema exactly), `args.rs` (typed, tolerant
  argument accessors).
- `config.rs` : full `ServerConfig` (same keys and defaults as the C#) + `${VAR}` expansion.
- `errors.rs` : the 14 `ERR_*` codes, `ToolError`, HTTP status mapping.
- `identity.rs` : RS256 verification, 30s clock skew, forwarded header then `Authorization`.
- `safety.rs` : path normalization, read-before-write, write quota, capped audit, trash.
- `storage/` : `sqlite.rs` (serialized, WAL, offloaded), `meta.rs` (nodes + blob_refs,
  content-addressing, refcount GC), `admin.rs` (project/project_member, caseless ACL,
  project id validation), `blob/local.rs` + `blob/s3.rs`, `volume.rs` (VolumeClient),
  `mod.rs` (backend factories + StoreManager cache).
- `core/` : `fs_ops.rs` (every filesystem operation, the engine the tools call), `diff.rs`.
- `docs/` : `extract.rs`, `docx.rs`, `symbols.rs` (tree-sitter + lexical fallback),
  `ocr.rs` (pluggable, null by default), `mime.rs`.
- `tools/` : one module per family, each `register(&mut ToolRegistry)`. `all.rs` has
  `register_all`.
- `api/` : `dataplane.rs` (the `/api/fs` routes), `openapi.rs` (spec + Swagger UI).
- `git/` : `db.rs` (SQLite index), `odb.rs` (objects in the blob store under `git:{sha}`),
  `repo.rs` (per project repository + write lock), `http/` (pkt-line, upload-pack,
  receive-pack), `oauth/` (store, AES-GCM cipher, encrypted persistence, device flow).
- `crates/parity-harness/` : differential tester (`corpus.rs`, `normalize.rs`).
- `crates/agent/` : the interactive CLI agent (an MCP **client**, not part of the server).
  `mcp.rs` (stateless JSON-RPC, fuzzy tool name resolution), `llm.rs` (OpenAI compatible
  streaming with tool calling), `input.rs` (wrap aware line editor), `ui.rs` (markdown to
  ANSI), `spinner.rs`, `session.rs`. Config: `config/agent_test.yaml`.
- `TOOL_CONTRACT.txt` : the 55 tool schemas and return shapes captured from the LIVE C#
  server. **This is the authoritative contract**; it beats reading the C# source.
- `parity-golden.json` : captured C# baseline for the harness.

## Conventions
- `mount_id` is a required parameter on every `fs.*` and `git.*` tool. Errors are
  `ToolError::<code>(msg)` carrying a stable `ERR_*`.
- Tool parameter names are snake_case so the generated JSON schema matches the C#.
  Parameter descriptions are the LLM-facing docs and are compared for parity: copy them
  verbatim from `TOOL_CONTRACT.txt`.
- Every `fs.*`/`git.*` handler: `state.authorize(mount_id, person)` first, then normalize
  every path through `state.safety.normalize_path`, then call the engine in `core::fs_ops`.
  Never reimplement an operation in the tool layer, and never in `api/dataplane.rs`
  either: the MCP surface and the REST plane MUST share one implementation, otherwise a
  fix on one path silently leaves the other on the old behaviour. `core::fs_ops` is the
  only place an operation is written; both layers are thin adapters over it.
- Platform admin manages projects and membership; it does NOT get implicit file access.
  Keep that separation.
- SQLite is accessed only through `storage::sqlite::SqliteDb` (one connection, serialized,
  offloaded to the blocking pool). Never block a request thread on the database.
- Files are content-addressed: write puts the blob then the node, copy is metadata only,
  delete GCs the blob at refcount 0, an empty file stores no blob. The local layout
  `{root}/{sha[..2]}/{sha}` is part of the on-disk contract.
- Secrets come from the environment only (`${VAR}` expansion in YAML, or read at the
  composition root). Never log a token or a key.
- Comments explain WHY. No dashes as punctuation anywhere in code, comments or output.
- Adding a storage backend: implement the trait in `storage/traits.rs` plus a branch in
  `storage/mod.rs`.
- Glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.
  It is Python `fnmatch` semantics, so a single `*` DOES cross a '/' boundary (verified
  against the reference: `*.rs` matches `/src/nested.rs`). Do not reach for `globset` with
  `literal_separator`, it would silently narrow `fs.glob` and `fs.grep`.

## Quality gate
`cargo test --workspace` and `cargo clippy --all-targets -- -D warnings` must both be
clean before any commit. The parity harness must show no unjustified difference.

## Deliberate divergences from the C#
Reproducing these exactly would reproduce a defect; each is documented at its call site
and listed in `README.md`: `unpack ok` in the push report, membership enforced on git
routes, `max_pack_size_mb` enforced, pushed objects really indexed, no custom libgit2 ODB
backend (blob store is the source of truth, bytes identical), `ERR_NOT_SUPPORTED` for an
unsupported extraction format, numbered list markers kept in generated docx.

## Documentation index
- `.agent_docs/architecture.md` : storage model, request lifecycle, safety, error logging.
- `.agent_docs/tools.md` : the 55 tool reference (families, parameters, authorization).
- `.agent_docs/api.md` : the `/api/fs` REST plane and the OpenAPI single source of truth.
- `.agent_docs/git.md` : git objects in the blob store, HTTP smart protocol, OAuth.
- `.agent_docs/config.md` : full YAML schema, env expansion, secrets.
- `.agent_docs/testing.md` : test layout, how to run the parity harness, MinIO opt-in.
- `.agent_docs/parity.md` : what parity means here, how it is verified, known divergences.
- `.agent_docs/agent.md` : the CLI agent, its config, and the terminal invariants it depends on.
