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

**134 steps compared, 13 differences, every one deliberate.** The instruction is now to
fix what can be fixed on the Rust side rather than mirror a defect, so this list is a
changelog of intentional improvements, not a parity debt.

### Errors carry a usable code and a usable status

| Step(s) | Reference | Here | Why |
|---|---|---|---|
| `read_missing`, `stat_missing`, `read_a_dir`, `traversal`, `missing_required_arg` | `"An error occurred invoking 'fs.read'."` with **no code** | `ERR_NOT_FOUND` / `ERR_INVALID_ARGUMENT` | The reference storage layer raises a bare `IOException`, which is not an `McpException`, so the SDK emits a generic sentence. A client cannot tell a missing file from a bad argument from a crash. |
| `rest_missing` | HTTP 500 | HTTP 404 | Same leak on the REST plane: an absent file was reported as a server failure. |
| `rest_edit_no_match`, `rest_edit_ambiguous` | HTTP 400 | HTTP 422 | The reference maps six codes and defaults the rest to a generic 400 (`GetValueOrDefault(code, 400)`), so "the text is not there" looked like "your request is malformed". 422 says the request was fine and the content was not. |
| `rest_extract_unsupported` | HTTP 400, `ERR_INVALID_ARGUMENT` | HTTP 501, `ERR_NOT_SUPPORTED` | Asking to extract an `.mp3` is not a malformed request, it is a capability this server does not have. |
| (not in the corpus) quota, read guard, duplicate project | HTTP 400 for all three | 429, 428, 409 | The status is the first thing a caller branches on: a spent budget, a missing precondition and a name conflict are three different situations with three different remedies. |

Note for the record: an earlier draft of this document claimed the reference sent unmapped
codes to 500. That was wrong, it defaults to 400. The harness caught it.

### Behaviour that was simply incorrect

| Step(s) | Reference | Here | Why |
|---|---|---|---|
| `rest_list_a_file` | HTTP 200 with `{"entries": []}` | HTTP 400 `ERR_INVALID_ARGUMENT` | Listing a file is a caller mistake. Answering 200 with an empty listing invents a directory that does not exist and hides the bug. |
| `rest_delete` | `{"deleted": true}`, no trash, no audit, a whole tree removed without `recursive` | the tool payload `{path, trashed, trash_path}`, honouring `recursive` and `trash` | The REST route called the volume client directly, so the same delete through two doors behaved differently and the REST door was the destructive one: it skipped the trash, ignored `safety.allow_hard_delete`, needed no `recursive`, and left no audit entry. `move` had the same shape of bug (no no clobber). Both now go through the engine. |
| `upload` | no quota charge, no audit entry | charged and audited like every write | The highest volume write path was the only one with no accounting: a caller could push unlimited bytes and leave no trace. Verified live: a 50 byte upload against a 30 byte quota is now 429. |
| `mkdir` (REST) | `parents` and `exist_ok` ignored, no audit entry | same parameters and audit as the tool | It called the volume client directly. |
| (unit tested) `git.remote_clone` | wrote every file with no quota charge and no audit entry | the whole import is charged up front, then audited | An import that does not fit must leave the volume untouched rather than half populated, so the total is charged before the first write. A large repository therefore needs `safety.write_quota_bytes` raised, which is the honest trade: a bulk write is still a write. Verified live: a 900 byte repository against a 500 byte quota is refused and the volume stays empty. |
| (unit tested) `git.checkout_file` | audited, but not charged | charged too | Restoring a file from history is a write. |
| (unit tested) `fs.move` with `overwrite: true` | always failed with `ERR_NO_CLOBBER` | replaces the destination | The flag was checked and then ignored: the metadata store refuses to rename onto an existing path, so `overwrite` was dead code on both surfaces. The destination is now cleared first, GCing what it referenced. |
| `find_refs_py` | references ordered `[3, 2]` | `[2, 3]` | The reference order is a tree-sitter traversal artifact. Ascending by line is deterministic and useful. |
| (unit tested) `fs.tree` at exactly the node cap | one node short, flagged `truncated` | complete, `truncated: false` | The cap was checked after incrementing, so the cap-th node was dropped and a tree that fitted was reported as incomplete. |
| `swagger_json` | `/api/fs/roots` absent | documented | ASP.NET excludes terminal `RequestDelegate` handlers from its OpenAPI document, so the reference page hides a route it serves. |

### Not reachable by the corpus, listed for completeness

- **The git protocol actually works.** Three reference defects, each verified against both
  servers, made a real `git clone` or `git push` impossible: `upload-pack` did not
  advertise `multi_ack_detailed` (required over smart HTTP); the pack was built with
  `insert_recursive`, which omits a commit's ancestry, so any repository with more than one
  commit produced an incomplete pack; and the `receive-pack` report was raw pkt-lines even
  when the client negotiated `side-band-64k`, so git aborted with `bad band #117` after the
  push had landed. Fixed with the detailed capability, a revwalk fed to `insert_walk`, and
  band 1 framing. Verified end to end: clone, commit, push, reclone.
- `receive-pack` sends the `unpack ok` report line the protocol requires.
- Git HTTP routes enforce project membership; the reference only checked that the repo
  existed, so any verified token could read or write any project.
- `git.max_pack_size_mb` is enforced; the reference parsed it and never used it.
- Pushed objects are really indexed and imported; the reference path was a stub.
- `auth.jwt.algorithms` is honoured. The reference parsed the key and hardcoded RS256, so a
  configured policy was silently ignored. Unsupported names are logged at startup, and the
  HMAC family is refused on purpose (an `HS*` algorithm with a public key file would let
  anyone holding that key mint tokens).
- No custom libgit2 ODB backend (`git2` cannot express one from safe Rust): the blob store
  is the source of truth and is synced around libgit2 calls. Stored bytes are identical.
- Generated `.docx` keeps numbered list markers and renders fenced code as monospaced
  paragraphs instead of leaking the backtick lines.
- `is_owner` and `is_admin` flags are computed caselessly, like the checks that authorize.
- An invalid `fs.grep` regex is `ERR_INVALID_ARGUMENT`; the reference let the exception
  escape into a generic internal error.
- One implementation per operation, shared by the MCP surface and the REST plane
  (`core::fs_ops`), including the V4A patch engine, which used to live in the tool layer and
  force the REST route to dispatch back through the tool registry.

## Working on parity

- When a behaviour is unclear, **ask the running reference server**, do not read its
  source and guess. That is how the wire framing, the `mode` string (`"0o644"`), the
  read-guard being satisfied by a write, and the sharded blob layout were all pinned.
- `TOOL_CONTRACT.txt` is the captured tool surface and beats the C# source on conflict.
- Recapture the golden after touching the corpus, and commit it with the change.
- If you must diverge, it has to be because parity would reproduce a defect. Document it
  at the call site, in this table, and in `README.md`.
