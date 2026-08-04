# Parity: what it means here and how it is verified

The prime directive of this port is **strict 1:1 external parity** with the C#
implementation (`../mcp-fs-csharp`): an existing MCP, REST or git client must not be able
to tell the two servers apart, and a volume written by one must be readable by the other.

## What is inside the contract

| Surface | Pinned |
|---|---|
| MCP tools | the 55 names, every parameter name (snake_case), type, default and `required` list, and every parameter description (they are the LLM facing docs) |
| MCP wire | `Content-Type: text/event-stream`, `event: message\ndata: {json}\n\n` framing, the `result`/`id`/`jsonrpc` key order, `initialize` capabilities, 401 as JSON, 202 for notifications, `-32602` unknown tool, `-32601` unknown method |
| Tool results | every key of every returned object |
| Errors | the 14 `ERR_*` codes and their HTTP mapping (403/404/409/400/401) |
| Storage | the `nodes`, `blob_refs`, `project`, `project_member` schemas; content addressing by sha256; refcount GC at 0; an empty file storing no blob; the local blob layout `{root}/{sha[..2]}/{sha}`; the S3 layout `bucket mcpfs-{project}`, key = sha256; git objects under key `git:{sha}` in the `{type} {len}\0{payload}` format |
| REST | every route and query/body parameter name, snake_case JSON |
| Git | the smart HTTP wire protocol (pkt-line, upload-pack, receive-pack) |
| Config | every YAML key, its default, and `${VAR}` / `${VAR:-default}` expansion |
| CLI | `serve`, `keys`, `token`, `version` |

## What is NOT inside the contract

Deliberately not compared, because nothing observable depends on it:

- **`tools/list` ordering.** Clients look a tool up by name. The set and each tool's
  schema are compared strictly; the registration order is an implementation detail on
  both sides.
- **Wall clock values**: `mtime`, `ctime`, `atime`, `timestamp`, `created_at`,
  `added_at`, `expires_at`, the epoch embedded in a trash path, the server version.
- **Host paths** in a message.
- **The wording of an error sentence.** The `ERR_*` code is what a client matches on, so
  the harness reduces an error text to `tool + code`. A different code still fails.
- **OpenAPI component schema names** (`MkdirBody` vs `MkdirBody2`): the shape is
  compared, the generated name is not.
- **Environment**: projects and users that exist on one server from real use.

## How it is verified

Three layers, from cheapest to strongest.

**1. Schema equality tests (unit).** `tools/mod.rs` and `tools/all.rs` compare all 55
descriptions and `inputSchema` values against `parity-golden.json`, *serialized*, so even
property key order drift fails the build. This is the fastest guard and it runs on every
`cargo test`.

**2. The differential harness (integration).** `crates/parity-harness` replays a 128 step
corpus (`corpus.rs`) covering the MCP surface, the REST plane, every error path, the four
tolerant string-array shapes, unicode, special characters and boundary values. It runs in
two modes so the two servers never need to be up together:

```bash
cargo run -p parity-harness -- capture --base http://127.0.0.1:5002 --token "$CS"   --owner admin@example.com --out parity-golden.json
cargo run -p parity-harness -- compare --base http://127.0.0.1:5003 --token "$RUST" --owner admin@example.com --golden parity-golden.json
```

`parity-golden.json` is the committed C# baseline. A fresh project id is generated per
run (tearing a project down on the reference leaves its volume behind, so a reused id
would hit `ERR_NO_CLOBBER` on the second replay) and the id is masked in the recorded
output so two runs are comparable.

**3. Live protocol checks.** The wire contract in `mcp/mod.rs` was captured with `curl`
against the running C# server, not inferred from its source, and the framing assertions
in `app.rs` encode it.

## Current result

**128 steps compared, 8 differences, every one of them deliberate.** Each is a case where
copying the reference exactly would copy a defect.

| Step(s) | Reference | Here | Why we diverge |
|---|---|---|---|
| `read_missing`, `stat_missing`, `read_a_dir`, `traversal`, `missing_required_arg` | `"An error occurred invoking 'fs.read'."` with **no code** | `ERR_NOT_FOUND` / `ERR_INVALID_ARGUMENT` | The C# storage layer raises a bare `IOException`, which is not an `McpException`, so the SDK emits a generic sentence. A client cannot tell a missing file from a bad argument from a crash. Verified live on a project the caller owns. |
| `rest_missing` | HTTP 500 | HTTP 404 | Same leak on the REST plane. The spec (FR-061) maps `ERR_NOT_FOUND` to 404. |
| `swagger_json` | `/api/fs/roots` absent | documented | ASP.NET excludes terminal `RequestDelegate` handlers from its OpenAPI document, so the reference page silently hides a route it serves. |
| `find_refs_py` | references ordered `[3, 2]` | `[2, 3]` | The reference order is a tree-sitter traversal artifact (deterministic but descending). Ascending by line is deterministic and useful. |

Divergences that the corpus does not reach, listed here for completeness (all in
`README.md` too, and documented at their call site):

- **The git protocol is actually usable.** Three reference defects, each verified against
  both servers, made a real `git clone` or `git push` impossible:
  `upload-pack` did not advertise `multi_ack_detailed`, which git requires over smart HTTP;
  the pack was built with `insert_recursive`, which omits a commit's ancestry, so any
  repository with more than one commit produced an incomplete pack; and the `receive-pack`
  report was sent as raw pkt-lines even when the client negotiated `side-band-64k`, so git
  aborted with `bad band #117` after the push had landed. Fixed with the detailed
  capability, a revwalk fed to `insert_walk`, and band 1 framing. Verified end to end:
  clone, commit, push, reclone, three commits and exact content.
- `receive-pack` sends the `unpack ok` report line the protocol requires; the C# omits it,
  so a real `git push` reports a failure even though the refs update.
- Git HTTP routes enforce project membership; the C# only checked that the repo existed,
  so any verified token could read or write any project.
- `git.max_pack_size_mb` is enforced; the C# parsed it and never used it.
- Pushed objects are really indexed and imported; the C# path was a stub.
- No custom libgit2 ODB backend (`git2` cannot express one from safe Rust): the blob store
  is the source of truth and is synced around libgit2 calls. Stored bytes are identical;
  the on-disk object directory becomes a rebuildable cache.
- An unsupported extraction format returns `ERR_NOT_SUPPORTED` rather than
  `ERR_INVALID_ARGUMENT` (message text unchanged).
- Generated `.docx` keeps numbered list markers and renders fenced code as monospaced
  paragraphs instead of leaking the backtick lines.
- `is_owner` and `is_admin` flags are computed caselessly, like the checks that actually
  authorize; the C# used ordinal comparisons there and could report a value contradicting
  its own gate.

## Working on parity

- When a behaviour is unclear, **ask the running reference server**, do not read its
  source and guess. That is how the wire framing, the `mode` string (`"0o644"`), the
  read-guard being satisfied by a write, and the sharded blob layout were all pinned.
- `TOOL_CONTRACT.txt` is the captured tool surface and beats the C# source on conflict.
- Recapture the golden after touching the corpus, and commit it with the change.
- If you must diverge, it has to be because parity would reproduce a defect. Document it
  at the call site, in this table, and in `README.md`.
